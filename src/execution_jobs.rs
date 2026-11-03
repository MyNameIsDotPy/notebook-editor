//! Bounded, root-scoped background execution handles for MCP clients.
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

#[derive(Default, Clone)]
pub struct Jobs(Arc<Mutex<HashMap<String, Arc<Mutex<Job>>>>>);
struct Job {
    cancel: Arc<AtomicBool>,
    sequence: u64,
    events: VecDeque<Value>,
    result: Option<tempfile::NamedTempFile>,
    failure: Option<String>,
    finished: bool,
}
impl Job {
    fn event(&mut self, value: Value) {
        self.sequence += 1;
        let value = if value.to_string().len() > 65536 {
            json!({"type":"output", "truncated":true})
        } else {
            value
        };
        self.events
            .push_back(json!({"sequence":self.sequence, "event":value}));
        while self.events.len() > 128 {
            self.events.pop_front();
        }
    }
}

impl Jobs {
    pub fn start(&self, work: impl FnOnce() -> Value + Send + 'static) -> Result<String> {
        let id = crate::commands::session_client::new_token();
        let job = Arc::new(Mutex::new(Job {
            cancel: Arc::new(AtomicBool::new(false)),
            sequence: 0,
            events: VecDeque::new(),
            result: None,
            failure: None,
            finished: false,
        }));
        let mut jobs = self.0.lock().unwrap();
        // Evict completed jobs only. Never evict a running execution.
        if jobs.len() >= 32 {
            let key = jobs
                .iter()
                .find(|(_, j)| j.lock().unwrap().finished)
                .map(|(k, _)| k.clone());
            if let Some(key) = key {
                jobs.remove(&key);
            } else {
                bail!("Execution queue is full");
            }
        }
        job.lock().unwrap().event(json!({"type":"queued"}));
        jobs.insert(id.clone(), job.clone());
        drop(jobs);
        let cancel = job.lock().unwrap().cancel.clone();
        std::thread::spawn(move || {
            let sink = job.clone();
            job.lock().unwrap().event(json!({"type":"started"}));
            crate::commands::run::observe_with(cancel, move |event| {
                sink.lock().unwrap().event(event.clone());
            });
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                .unwrap_or_else(
                    |_| json!({"status":"driver_failure", "message":"Execution worker panicked"}),
                );
            let stored = (|| -> Result<tempfile::NamedTempFile> {
                let mut file = tempfile::NamedTempFile::new()?;
                serde_json::to_writer(file.as_file_mut(), &result)?;
                Ok(file)
            })();
            let mut job = job.lock().unwrap();
            match stored {
                Ok(file) => job.result = Some(file),
                Err(e) => job.failure = Some(e.to_string()),
            }
            job.finished = true;
            job.event(json!({"type":"finished", "status":result.get("status")}));
        });
        Ok(id)
    }
    /// Orderly stdio disconnects cancel outstanding work before process exit.
    pub fn shutdown(&self) {
        let jobs: Vec<_> = self.0.lock().unwrap().values().cloned().collect();
        for job in &jobs {
            job.lock().unwrap().cancel.store(true, Ordering::SeqCst);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline
            && jobs.iter().any(|j| !j.lock().unwrap().finished)
        {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    pub fn get(&self, id: &str, after: u64, cancel: bool) -> Result<Value> {
        let job = self
            .0
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .context("Unknown or expired execution ID")?;
        let job = job.lock().unwrap();
        if cancel {
            job.cancel.store(true, Ordering::SeqCst);
        }
        let result = match &job.result {
            Some(file) => serde_json::from_reader::<_, Value>(std::fs::File::open(file.path())?)?,
            None => json!(null),
        };
        Ok(
            json!({"execution_id":id, "state":if job.finished { "finished" } else { "running" }, "result":result, "message":job.failure, "sequence":job.sequence, "first_sequence":job.events.front().and_then(|e| e["sequence"].as_u64()).unwrap_or(job.sequence+1), "events":job.events.iter().filter(|e| e["sequence"].as_u64().unwrap_or(0)>after).collect::<Vec<_>>()}),
        )
    }
}
