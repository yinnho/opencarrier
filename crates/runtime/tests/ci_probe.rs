//! TEMPORARY CI probe (bisect branch only - do NOT merge).
//!
//! Empirically isolates which operation in `ProcessManager::start` kills the
//! GitHub-hosted runner: env_clear, process_group(0), or the tokio spawn path.
//! Each probe runs in its own CI step; the step where the runner dies is the
//! culprit operation.

use std::process::{Command, Stdio};

fn path_env() -> String {
    std::env::var("PATH").unwrap_or_default()
}

/// Control: plain piped spawn, no sandboxing at all.
#[test]
fn probe_plain_spawn() {
    println!("PROBE plain_spawn: spawning cat...");
    let mut child = Command::new("cat")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cat");
    println!("PROBE plain_spawn: spawned pid={}", child.id());
    let _ = child.kill();
    let _ = child.wait();
    println!("PROBE plain_spawn: done");
}

/// env_clear + re-add PATH only.
#[test]
fn probe_env_clear() {
    println!("PROBE env_clear: spawning cat...");
    let mut child = Command::new("cat")
        .env_clear()
        .env("PATH", path_env())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cat");
    println!("PROBE env_clear: spawned pid={}", child.id());
    let _ = child.kill();
    let _ = child.wait();
    println!("PROBE env_clear: done");
}

/// process_group(0) only - new process group via pre_exec setpgid.
#[cfg(unix)]
#[test]
fn probe_process_group() {
    use std::os::unix::process::CommandExt;
    println!("PROBE process_group: spawning cat...");
    let mut child = Command::new("cat")
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cat");
    println!("PROBE process_group: spawned pid={}", child.id());
    let _ = child.kill();
    let _ = child.wait();
    println!("PROBE process_group: done");
}

/// Full tokio path - exactly what ProcessManager::start does.
#[tokio::test]
async fn probe_tokio_full_path() {
    println!("PROBE tokio_full: spawning cat via ProcessManager...");
    let pm = runtime::process_manager::ProcessManager::new(5);
    let id = pm
        .start("probe", "cat", &[], None, None)
        .await
        .expect("start cat");
    println!("PROBE tokio_full: started id={id}");
    let _ = pm.kill(&id).await;
    println!("PROBE tokio_full: done");
}

/// Granular kill-path probe: replicate kill_tree_unix step by step with
/// markers, so the last marker before death pinpoints the killing operation.
#[cfg(unix)]
#[tokio::test]
async fn probe_kill_granular() {
    println!("KPROBE a: spawning cat with process_group(0)...");
    let mut cmd = tokio::process::Command::new("cat");
    cmd.env_clear()
        .env("PATH", path_env())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .process_group(0);
    let mut child = cmd.spawn().expect("spawn cat");
    let pid = child.id().expect("pid");
    println!("KPROBE b: spawned pid={pid}");
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    println!("KPROBE c: running /bin/kill -TERM -{pid}...");
    let out = tokio::process::Command::new("kill")
        .args(["-TERM", &format!("-{pid}")])
        .output()
        .await
        .expect("run kill");
    println!(
        "KPROBE d: group kill done status={:?} stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );

    println!("KPROBE e: kill -0 {pid} liveness check...");
    let check = tokio::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .await
        .expect("kill -0");
    println!("KPROBE f: check status={:?}", check.status.code());

    println!("KPROBE g: child.kill() (direct SIGKILL)...");
    let _ = child.kill().await;
    let _ = child.wait().await;
    println!("KPROBE h: done");
}
