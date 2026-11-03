"""Real-kernel regressions. Run with a Python containing nbclient/nbformat/ipykernel.
Build both binaries first with cargo build. No user kernels or registry are modified.
"""
import concurrent.futures
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / 'target' / 'debug' / ('nbedit.exe' if os.name == 'nt' else 'nbedit')

class ExecutionTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(dir=ROOT / 'target')
        def cleanup_tmp():
            # Windows can retain process cwd/history handles briefly after exit.
            for attempt in range(30):
                try:
                    self.tmp.cleanup()
                    return
                except PermissionError:
                    if attempt == 29: raise
                    time.sleep(.1)
        self.addCleanup(cleanup_tmp)
        self.dir = Path(self.tmp.name)
        self.env = os.environ | {'JUPYTER_PATH': str(self.dir), 'JUPYTER_RUNTIME_DIR': str(self.dir / 'runtime'), 'IPYTHONDIR': str(self.dir / 'ipython'), 'LOCALAPPDATA': str(self.dir), 'XDG_DATA_HOME': str(self.dir), 'TMP': str(self.dir), 'TEMP': str(self.dir)}
        spec = self.dir / 'kernels' / 'test'
        spec.mkdir(parents=True)
        (spec / 'kernel.json').write_text(json.dumps({'argv':[sys.executable, '-m', 'ipykernel_launcher', '-f', '{connection_file}'], 'display_name':'Test', 'language':'python'}))

    def notebook(self, sources):
        path = self.dir / 'test.ipynb'
        path.write_text(json.dumps({'nbformat':4, 'nbformat_minor':5, 'metadata':{}, 'cells':[{'id':f'cell-{i}', 'cell_type':'code', 'source':[s], 'metadata':{}, 'execution_count':99, 'outputs':[{'output_type':'error', 'ename':'OldError', 'evalue':'stale', 'traceback':[]}]} for i,s in enumerate(sources)]}))
        return path

    def run_cli(self, path, selection='all', *args):
        return subprocess.run([str(BIN), 'run', str(path), selection, '--kernel', 'test', '--driver-python', sys.executable, '--json', *args], env=self.env, capture_output=True, text=True, timeout=45)

    def daemon(self):
        source = f'KERNEL_NAME="test"\nTOKEN="secret"\nSTARTUP_TIMEOUT=15\n' + (ROOT / 'src/commands/session_daemon.py').read_text()
        p = subprocess.Popen([sys.executable, '-u', '-c', source], cwd=self.dir, env=self.env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.addCleanup(self.stop_daemon, p)
        with concurrent.futures.ThreadPoolExecutor() as pool:
            ready = json.loads(pool.submit(p.stdout.readline).result(timeout=25))
        self.assertEqual(ready['status'], 'ready', ready)
        self.port = ready['port']
        return p

    def stop_daemon(self, p):
        if p.poll() is None:
            try:
                self.request(cmd='shutdown')
                p.wait(timeout=10)
            except Exception:
                p.kill()
                p.wait()
        p.stdout.close()
        errors = p.stderr.read()
        if "Traceback" in errors: print(errors, file=sys.stderr)
        p.stderr.close()

    def request(self, **data):
        with socket.create_connection(('127.0.0.1', self.port), timeout=10) as s:
            s.sendall((json.dumps(data | {'token':'secret'})+'\n').encode())
            with s.makefile() as f:
                result = json.loads(f.readline())
        self.assertNotEqual(result.get('status'), 'error', result)
        return result

    def submit(self, *sources, **options):
        return self.request(cmd='submit', cells=[{'id':str(i), 'source':s, 'metadata':{}} for i,s in enumerate(sources)], **options)['execution_id']

    def result(self, execution_id):
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            r = self.request(cmd='result', execution_id=execution_id)
            if r['state'] == 'finished':
                return r
            time.sleep(.1)
        self.fail('Execution did not finish')

    def test_early_failure_and_context_results(self):
        path = self.notebook(['x=1', '1/0', 'print(x)'])
        p = self.run_cli(path)
        self.assertEqual(p.returncode, 1, p.stderr)
        r = json.loads(p.stdout)
        self.assertEqual(r['executed_cells'], [1,2])
        self.assertEqual(r['failed_cell'], 2)
        self.assertEqual([c['state'] for c in r['cells']], ['completed','failed','skipped'])
        self.assertEqual(r['cells'][2]['outputs'], [])
        self.assertEqual(json.loads(path.read_text())['cells'][2]['outputs'][0]['ename'], 'OldError')
        p = self.run_cli(path, '3', '--include-prior')
        r = json.loads(p.stdout)
        self.assertEqual(r['failed_cell'], 2)
        self.assertTrue(r['cells'][1]['context'])
        self.assertEqual(r['cells'][1]['outputs'][0]['ename'], 'ZeroDivisionError')

    def test_allow_errors_and_cell_timeout(self):
        path = self.notebook(['1/0', 'print(42)'])
        p = self.run_cli(path, 'all', '--allow-errors')
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(json.loads(p.stdout)['status'], 'ok_with_errors')
        path = self.notebook(['import time; time.sleep(20)', '42'])
        p = self.run_cli(path, 'all', '--timeout', '1')
        r = json.loads(p.stdout)
        self.assertEqual(r['status'], 'cell_timeout', r)
        self.assertEqual(r['failed_cell'], 1)
        self.assertEqual(r['cells'][1]['state'], 'skipped')

    def test_queue_state_output_cancel_and_restart(self):
        self.daemon()
        first = self.submit('import time; x=10; print("live", flush=True); time.sleep(2)')
        second = self.submit('print(x+1)')
        self.assertIn(self.request(cmd='result', execution_id=second)['state'], ['queued','running'])
        self.assertEqual(self.result(first)['status'], 'ok')
        r = self.result(second)
        self.assertEqual(r['status'], 'ok', r)
        self.assertIn('11', r['results'][0]['outputs'][0]['text'])
        job = self.submit('print("before", flush=True); time.sleep(20)')
        time.sleep(1)
        r = self.request(cmd='result', execution_id=job)
        self.assertTrue(any(e['type']=='output' for e in r['events']), r)
        self.request(cmd='cancel', execution_id=job)
        self.assertEqual(self.result(job)['status'], 'interrupted')
        self.request(cmd='restart')
        self.assertEqual(self.result(self.submit('print(x)'))['status'], 'cell_error')

    def test_deadlines_cancel_queued_and_active_jobs(self):
        self.daemon()
        active = self.submit('import time; print("checkpoint", flush=True); time.sleep(20)', overall_timeout=2)
        queued = self.submit('raise RuntimeError("must not run")', overall_timeout=.2)
        self.assertEqual(self.result(queued)['status'], 'overall_timeout')
        r = self.result(active)
        self.assertEqual(r['status'], 'overall_timeout', r)
        self.assertIn('checkpoint', r['results'][0]['outputs'][0]['text'])
        state = self.request(cmd='status')['state']
        if state == 'unhealthy': self.request(cmd='restart')
        self.assertEqual(self.result(self.submit('2+2'))['status'], 'ok')

    def test_session_cell_timeout_requires_recovery_and_blocks_later_code(self):
        self.daemon()
        job = self.submit('import time; time.sleep(30)', timeout=1)
        self.assertEqual(self.result(job)['status'], 'cell_timeout')
        self.assertEqual(self.request(cmd='status')['state'], 'unhealthy')
        self.request(cmd='restart')
        self.assertEqual(self.result(self.submit('print(123)'))['status'], 'ok')

    def test_idle_expiry_and_bounded_history(self):
        p = self.daemon()
        for _ in range(35):
            self.result(self.submit('1'))
        r = self.request(cmd='status')
        self.assertLessEqual(len(r['history']), 32)
        self.request(cmd='configure', idle_timeout=.2)
        p.wait(timeout=10)

    def test_save_conflict_preserves_external_edit(self):
        path = self.notebook(['import time; time.sleep(3); print(1)'])
        p = subprocess.Popen([str(BIN), 'run', str(path), 'all', '--kernel','test','--driver-python',sys.executable,'--json'], env=self.env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        time.sleep(2)
        edited = json.loads(path.read_text()); edited['metadata']['external'] = True
        path.write_text(json.dumps(edited))
        stdout, stderr = p.communicate(timeout=30)
        r = json.loads(stdout)
        self.assertEqual(r['status'], 'save_conflict', stderr)
        self.assertFalse(r['outputs_saved'])
        self.assertTrue(json.loads(path.read_text())['metadata']['external'])
        self.assertTrue(r['cells'][0]['outputs'])

    def test_one_shot_overall_timeout_preserves_outputs_and_kills_kernel(self):
        import psutil
        path = self.notebook(['print("done")', 'import os,time; print(os.getpid(), flush=True); time.sleep(30)', '42'])
        p = self.run_cli(path, 'all', '--overall-timeout', '4')
        self.assertEqual(p.returncode, 124, p.stderr)
        r = json.loads(p.stdout)
        self.assertEqual(r['status'], 'overall_timeout')
        self.assertEqual(r['cells'][0]['state'], 'completed')
        self.assertEqual(r['cells'][2]['state'], 'skipped')
        text = r['cells'][1]['outputs'][0]['text']
        pid = int((''.join(text) if isinstance(text, list) else text).strip())
        time.sleep(.2)
        self.assertFalse(psutil.pid_exists(pid), f"Orphaned kernel {pid}")

    def mcp(self):
        binary = BIN.with_name('nbedit-mcp.exe' if os.name == 'nt' else 'nbedit-mcp')
        p = subprocess.Popen([str(binary), '--root', str(self.dir)], env=self.env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        def cleanup():
            p.stdin.close()
            p.wait(timeout=15)
            p.stdout.close(); p.stderr.close()
        self.addCleanup(cleanup)
        return p

    def tool(self, p, tool_name, **arguments):
        p.stdin.write(json.dumps({'jsonrpc':'2.0', 'id':1, 'method':'tools/call', 'params':{'name':tool_name,'arguments':arguments}})+'\n')
        p.stdin.flush()
        reply = json.loads(p.stdout.readline())['result']
        self.assertFalse(reply['isError'], reply)
        return reply['structuredContent']

    def test_mcp_background_events_results_and_cancellation(self):
        self.notebook(['import time; print("live", flush=True); time.sleep(20)'])
        p = self.mcp()
        started = self.tool(p, 'notebook_run_cells', path='test.ipynb', kernel='test', driver_python=sys.executable, background=True)
        job = started['execution_id']
        until = time.monotonic()+20
        saw_output = False
        while time.monotonic()<until:
            r = self.tool(p, 'notebook_execution_status', execution_id=job)
            if any('live' in json.dumps(e) for e in r['events']):
                saw_output = True
                break
            time.sleep(.1)
        self.assertTrue(saw_output)
        self.tool(p, 'notebook_execution_cancel', execution_id=job)
        while time.monotonic()<until:
            r = self.tool(p, 'notebook_execution_status', execution_id=job)
            if r['state']=='finished': break
            time.sleep(.1)
        self.assertEqual(r['state'], 'finished', r)
        self.assertEqual(r['result']['status'], 'interrupted', r)
        self.assertTrue(r['result']['outputs_saved'])

    def test_mcp_session_lifecycle_and_precise_results(self):
        self.notebook(['x=5', '1/0', 'print(x)'])
        p = self.mcp()
        self.tool(p, 'notebook_session_start', name='test-session', path='test.ipynb', kernel='test', driver_python=sys.executable)
        try:
            r = self.tool(p, 'notebook_run_cells', path='test.ipynb', session='test-session')
            self.assertEqual(r['status'], 'cell_error', r)
            self.assertEqual(r['failed_cell'], 2)
            self.assertEqual(r['executed_cells'], [1,2])
            self.assertEqual(r['cells'][2]['state'], 'skipped')
            self.assertEqual(self.tool(p, 'notebook_session_status', name='test-session')['state'], 'idle')
            self.tool(p, 'notebook_session_restart', name='test-session')
            r = self.tool(p, 'notebook_run_cells', path='test.ipynb', selection='3', session='test-session')
            self.assertEqual(r['status'], 'cell_error')
            self.assertEqual(r['cells'][0]['outputs'][0]['ename'], 'NameError')
        finally:
            self.tool(p, 'notebook_session_stop', name='test-session')

if __name__ == '__main__':
    unittest.main()
