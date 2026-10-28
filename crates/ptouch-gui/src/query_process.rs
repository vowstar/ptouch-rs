// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Bounded, read-only Bluetooth queries. Printing keeps its own job timing.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const OUTPUT_LIMIT: usize = 64 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(10);

struct QueryChild<'a>(&'a mut Child);

impl Drop for QueryChild<'_> {
    fn drop(&mut self) {
        // Every return path reaps the query, including reader/spawn errors.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn capture(
    pipe: impl Read + Send + 'static,
    tx: mpsc::Sender<(usize, std::io::Result<Vec<u8>>)>,
    stream: usize,
) -> Result<(), String> {
    std::thread::Builder::new()
        .name("bluetooth-query-output".into())
        .spawn(move || {
            let mut bytes = Vec::new();
            let result = pipe
                .take((OUTPUT_LIMIT + 1) as u64)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = tx.send((stream, result));
        })
        .map(|_| ())
        .map_err(|e| format!("Cannot read Bluetooth response: {e}"))
}

pub fn run(mut command: Command, timeout: Duration) -> Result<String, String> {
    let started = Instant::now();
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Cannot start Bluetooth query: {e}"))?;
    collect(&mut child, started, timeout)
}

fn collect(child: &mut Child, started: Instant, timeout: Duration) -> Result<String, String> {
    let child = QueryChild(child);
    let (tx, rx) = mpsc::channel();
    capture(
        child
            .0
            .stdout
            .take()
            .ok_or("Cannot read Bluetooth response.")?,
        tx.clone(),
        0,
    )?;
    capture(
        child
            .0
            .stderr
            .take()
            .ok_or("Cannot read Bluetooth response.")?,
        tx,
        1,
    )?;
    let mut output: [Option<Vec<u8>>; 2] = [None, None];
    let mut status = None;
    loop {
        while let Ok((stream, result)) = rx.try_recv() {
            let bytes = result.map_err(|e| format!("Cannot read Bluetooth response: {e}"))?;
            if bytes.len() > OUTPUT_LIMIT {
                return Err("Bluetooth response is larger than the size limit.".into());
            }
            output[stream] = Some(bytes);
        }
        if status.is_none() {
            status = child
                .0
                .try_wait()
                .map_err(|e| format!("Bluetooth query failed: {e}"))?;
        }
        if let Some(status) = status
            && output.iter().all(Option::is_some)
        {
            let [stdout, stderr] = output.map(Option::unwrap);
            if status.success() {
                return String::from_utf8(stdout)
                    .map_err(|_| "Bluetooth response has invalid text.".into());
            }
            let detail: String = String::from_utf8_lossy(&stderr)
                .trim()
                .chars()
                .take(512)
                .collect();
            return Err(if detail.is_empty() {
                format!("Bluetooth query failed ({status}).")
            } else {
                format!("Bluetooth query failed: {detail}")
            });
        }
        if started.elapsed() >= timeout {
            return Err("Bluetooth did not respond in time.".into());
        }
        // Reader threads own only bounded pipes, not printer state. Do not join
        // a pipe reader here: an inherited pipe must not extend the deadline.
        std::thread::sleep(POLL_INTERVAL.min(timeout.saturating_sub(started.elapsed())));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn child(mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args([
            "--exact",
            "query_process::tests::query_child",
            "--ignored",
            "--nocapture",
        ]);
        command.env("PTOUCH_QUERY_TEST_MODE", mode);
        command
    }

    #[test]
    #[ignore = "subprocess fixture for bounded query tests"]
    fn query_child() {
        match std::env::var("PTOUCH_QUERY_TEST_MODE").unwrap().as_str() {
            "ok" => {
                println!("query-result");
                eprintln!("native diagnostic");
            }
            "error" => {
                eprintln!("permission denied");
                std::process::exit(7);
            }
            "stdout-flood" => {
                let _ = std::io::stdout().write_all(&vec![b'x'; OUTPUT_LIMIT * 4]);
            }
            "stderr-flood" => {
                let _ = std::io::stderr().write_all(&vec![b'x'; OUTPUT_LIMIT * 4]);
            }
            "sleep" => std::thread::sleep(Duration::from_secs(30)),
            "invalid-text" => {
                let _ = std::io::stdout().write_all(&[0xff]);
            }
            _ => panic!("unknown fixture"),
        }
    }

    #[test]
    fn query_success_and_failure_capture_both_streams() {
        assert!(
            run(child("ok"), Duration::from_secs(10))
                .unwrap()
                .contains("query-result")
        );
        assert!(
            run(child("error"), Duration::from_secs(10))
                .unwrap_err()
                .contains("permission denied")
        );
        assert!(
            run(child("invalid-text"), Duration::from_secs(10))
                .unwrap_err()
                .contains("invalid text")
        );
    }

    #[test]
    fn query_output_is_bounded_on_both_streams() {
        for mode in ["stdout-flood", "stderr-flood"] {
            assert!(
                run(child(mode), Duration::from_secs(10))
                    .unwrap_err()
                    .contains("size limit")
            );
        }
    }

    #[test]
    fn query_timeout_returns_without_waiting_for_the_child_operation() {
        let mut process = child("sleep")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let start = Instant::now();
        let result = collect(&mut process, start, Duration::from_millis(100));
        let exited = process.try_wait().unwrap().is_some();
        // Avoid leaking a child even if this regression fails.
        if !exited {
            let _ = process.kill();
            let _ = process.wait();
        }
        assert!(result.unwrap_err().contains("in time"));
        assert!(exited, "query child still runs after the timeout returned");
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
