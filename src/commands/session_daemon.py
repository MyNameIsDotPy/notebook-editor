"""Embedded session protocol v2. All kernel access is serialized by worker()."""
import asyncio
import collections
import contextlib
import json
import os
import sys
import time
import uuid
import tempfile
from pathlib import Path

if sys.platform == "win32":
    asyncio.set_event_loop_policy(asyncio.WindowsSelectorEventLoopPolicy())
REQUEST_LIMIT = 1024 * 1024 * 1024
MAX_HISTORY = 32
MAX_EVENTS = 256
MAX_EVENT_BYTES = 1024 * 1024


def announce(payload):
    print(json.dumps(payload), flush=True)


async def main():
    import nbformat
    import nbclient
    nb = nbformat.v4.new_notebook()
    client = nbclient.NotebookClient(nb, kernel_name=KERNEL_NAME, startup_timeout=STARTUP_TIMEOUT)
    results_dir = tempfile.TemporaryDirectory(prefix="nbedit-results-")
    jobs = collections.OrderedDict()
    queue = asyncio.Queue()
    stopped = asyncio.Event()
    active = None
    healthy = True
    last_activity = time.time()
    idle_timeout = None
    timers = {}

    def event(job, kind, **data):
        job["sequence"] += 1
        entry = dict(sequence=job["sequence"], type=kind, time=time.time(), **data)
        # Large outputs stay available in the result; event snapshots are bounded.
        if len(json.dumps(entry)) > MAX_EVENT_BYTES:
            entry = dict(sequence=job["sequence"], type=kind, truncated=True)
        job["events"].append(entry)
        while len(job["events"]) > MAX_EVENTS or sum(len(json.dumps(e)) for e in job["events"]) > MAX_EVENT_BYTES:
            job["events"].pop(0)

    def prune():
        while len(jobs) > MAX_HISTORY:
            old = next((key for key, j in jobs.items() if j["state"] == "finished" and key != active), None)
            if old is None:
                break
            job = jobs.pop(old)
            if job.get("result_file"):
                Path(job["result_file"]).unlink(missing_ok=True)

    def snapshot(job):
        cell = job.get("cell")
        if cell is not None:
            result = job["results"][-1]
            result.update(outputs=cell.get("outputs", []), execution_count=cell.get("execution_count"), metadata=cell.get("metadata", {}))

    async def cancel(job, reason="interrupted"):
        if job["state"] == "finished":
            return
        if job["cancel"]:
            return
        job["cancel"] = reason
        if job["state"] == "running":
            await client.km.interrupt_kernel()
        else:
            job.pop("request", None)
            job.update(state="finished", status=reason, message=reason)
            event(job, "finished", status=reason)

    async def worker():
        nonlocal active, healthy, last_activity
        while True:
            job = await queue.get()
            if job["state"] == "finished":
                continue
            active = job["execution_id"]
            job.update(state="running", status="running")
            event(job, "started")
            req = job["request"]
            try:
                if not healthy:
                    raise RuntimeError("Kernel requires restart")
                client.allow_errors = bool(req.get("allow_errors", False))
                client.record_timing = bool(req.get("record_timing", True))
                client.timeout = None if req.get("timeout", -1) == -1 else req["timeout"]
                client.iopub_timeout = req.get("iopub_timeout", 4)
                for cell_req in req.get("cells", []):
                    if job["cancel"]:
                        break
                    cell = nbformat.v4.new_code_cell(cell_req["source"], metadata=cell_req.get("metadata", {}))
                    nb.cells[:] = [cell]
                    job["cell"] = cell
                    before_count = client.code_cells_executed
                    result = dict(id=cell_req["id"], state="running", outputs=[], execution_count=None, metadata=cell.metadata)
                    job["results"].append(result)
                    event(job, "cell_started", id=cell_req["id"])
                    task = asyncio.create_task(client.async_execute_cell(cell, 0, execution_count=client.code_cells_executed + 1))
                    previous = ""
                    interrupted_at = None
                    try:
                        while not task.done():
                            await asyncio.wait({task}, timeout=0.1)
                            snapshot(job)
                            current = json.dumps(cell.get("outputs", []))
                            if current != previous:
                                event(job, "output", id=cell_req["id"], outputs=cell.get("outputs", []))
                                previous = current
                            if job["cancel"]:
                                interrupted_at = interrupted_at or time.monotonic()
                                if time.monotonic() - interrupted_at > 5:
                                    task.cancel()
                                    with contextlib.suppress(BaseException):
                                        await task
                                    await client.km.shutdown_kernel(now=True)
                                    healthy = False
                                    break
                        if not task.cancelled():
                            await task
                        result["state"] = "skipped" if client.code_cells_executed == before_count else "failed" if any(o.get("output_type") == "error" for o in cell.outputs) or job["cancel"] else "completed"
                    except Exception as exc:
                        name = type(exc).__name__
                        result["state"] = "failed"
                        job.update(status="cell_timeout" if "Timeout" in name else "cell_error" if name == "CellExecutionError" else "kernel_error", failed_id=cell_req["id"], message=f"{name}: {exc}")
                        if name != "CellExecutionError":
                            # The output consumer has stopped. Never hand a kernel
                            # that may still be executing to the next queued job.
                            healthy = False
                            with contextlib.suppress(Exception):
                                await client.km.shutdown_kernel(now=True)
                    finally:
                        snapshot(job)
                        job.pop("cell", None)
                        event(job, "cell_completed", id=cell_req["id"], state=result["state"])
                    if result["state"] == "failed":
                        job["failed_id"] = job["failed_id"] or cell_req["id"]
                    if job["status"] not in ("running", "ok_with_errors") or job["cancel"]:
                        break
                    if result["state"] == "failed":
                        job["status"] = "ok_with_errors"
                job["status"] = job["cancel"] or ("ok" if job["status"] == "running" else job["status"])
            except Exception as exc:
                job.update(status="kernel_error", message=str(exc))
            finally:
                nb.cells.clear()
                # nbclient indexes display updates by cell; never retain prior batches.
                count = client.code_cells_executed
                client.reset_execution_trackers()
                client.code_cells_executed = count
                timer = timers.pop(job["execution_id"], None)
                if timer:
                    timer.cancel()
                job.pop("request", None)
                # Spill completed outputs to disk; history count and event memory are bounded.
                result_file = Path(results_dir.name) / (job["execution_id"] + ".json")
                try:
                    result_file.write_text(json.dumps(job["results"]), encoding="utf-8")
                    job.update(result_file=str(result_file), results=[])
                except OSError as exc:
                    job.update(status="driver_failure", message=f"Cannot retain results on disk: {exc}")
                job["state"] = "finished"
                event(job, "finished", status=job["status"])
                active = None
                last_activity = time.time()
                prune()

    async def deadline(job, seconds):
        await asyncio.sleep(seconds)
        await cancel(job, "overall_timeout")

    def public(job, after=0):
        snapshot(job)
        return {k: v for k, v in job.items() if k not in ("request", "cell", "cancel", "events", "result_file")} | {"results": json.loads(Path(job["result_file"]).read_text(encoding="utf-8")) if job.get("result_file") else job["results"], "events": [e for e in job["events"] if e["sequence"] > after], "first_sequence": job["events"][0]["sequence"] if job["events"] else job["sequence"] + 1}

    async def handle(reader, writer):
        nonlocal healthy, idle_timeout, last_activity
        try:
            req = json.loads(await reader.readline())
            if req.get("token") != TOKEN:
                raise ValueError("unauthorized")
            cmd = req.get("cmd")
            if cmd == "ping" or cmd == "status":
                if healthy and (client.km is None or not await client.km.is_alive()):
                    healthy = False
                response = dict(status="ok", protocol_version=2, state="busy" if active else "idle" if healthy else "unhealthy", active_execution=active, queue_length=sum(j["state"] == "queued" for j in jobs.values()), last_activity=last_activity, idle_timeout=idle_timeout, history=[{k: j[k] for k in ("execution_id", "state", "status", "created_at")} for j in jobs.values()])
            elif cmd == "submit":
                if not healthy:
                    raise ValueError("Kernel requires restart")
                if sum(j["state"] != "finished" for j in jobs.values()) >= MAX_HISTORY:
                    raise ValueError("Execution queue is full")
                if req.get("timeout", -1) < -1 or (req.get("overall_timeout") is not None and req["overall_timeout"] < 0):
                    raise ValueError("Invalid timeout")
                last_activity = time.time()
                job = dict(execution_id=uuid.uuid4().hex, state="queued", status="queued", results=[], failed_id=None, message=None, events=[], sequence=0, created_at=time.time(), request=req, cancel=None)
                jobs[job["execution_id"]] = job
                event(job, "queued")
                await queue.put(job)
                if req.get("overall_timeout") is not None:
                    timers[job["execution_id"]] = asyncio.create_task(deadline(job, req["overall_timeout"]))
                response = public(job)
            elif cmd in ("result", "cancel"):
                job = jobs.get(req.get("execution_id"))
                if job is None:
                    raise ValueError("Unknown or expired execution ID")
                if cmd == "cancel":
                    await cancel(job)
                response = public(job, req.get("after", 0))
            elif cmd == "interrupt":
                if active:
                    await cancel(jobs[active])
                response = dict(status="ok")
            elif cmd == "restart":
                if any(j["state"] != "finished" for j in jobs.values()):
                    raise ValueError("Cancel active and queued executions before restart")
                healthy = False
                if client.km is not None:
                    await client._async_cleanup_kernel()
                client.km = client.create_kernel_manager()
                await client.async_start_new_kernel()
                await client.async_start_new_kernel_client()
                client.reset_execution_trackers()
                healthy = True
                response = dict(status="ok")
            elif cmd == "configure":
                proposed = req.get("idle_timeout")
                if proposed is not None and proposed < 0:
                    raise ValueError("idle_timeout must be non-negative")
                idle_timeout = proposed
                last_activity = time.time()
                response = dict(status="ok", idle_timeout=idle_timeout)
            elif cmd == "shutdown":
                for job in list(jobs.values()):
                    await cancel(job)
                # Acknowledge shutdown only after the kernel process has exited.
                if client.km is not None and client.km.has_kernel:
                    await client.km.shutdown_kernel(now=True)
                stopped.set()
                response = dict(status="ok")
            else:
                raise ValueError(f"Unknown command: {cmd}")
        except Exception as exc:
            response = dict(status="error", message=f"{type(exc).__name__}: {exc}")
        try:
            writer.write((json.dumps(response) + "\n").encode())
            await writer.drain()
        finally:
            writer.close()
            await writer.wait_closed()

    async def idle_watch():
        while not stopped.is_set():
            await asyncio.sleep(0.25)
            if idle_timeout is not None and not any(j["state"] != "finished" for j in jobs.values()) and time.time() - last_activity >= idle_timeout:
                stopped.set()

    async with client.async_setup_kernel(cleanup_kc=True):
        server = await asyncio.start_server(handle, host="127.0.0.1", port=0, limit=REQUEST_LIMIT)
        announce(dict(status="ready", port=server.sockets[0].getsockname()[1], pid=os.getpid()))
        tasks = [asyncio.create_task(worker()), asyncio.create_task(idle_watch())]
        async with server:
            await stopped.wait()
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
    results_dir.cleanup()


try:
    asyncio.run(main())
except Exception as exc:
    announce(dict(status="error", message=f"{type(exc).__name__}: {exc}"))
