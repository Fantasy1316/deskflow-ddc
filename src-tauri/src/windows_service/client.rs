use super::{win::*, Command, Launch, Reply, MAX_MESSAGE};
use crate::engine::{Engine, Event};
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use std::os::windows::io::AsRawHandle;
use std::time::Duration;
use tokio::{
    net::windows::named_pipe::ClientOptions,
    sync::{mpsc, oneshot},
};
use tokio_util::codec::{Framed, LinesCodec};
use windows_sys::Win32::System::{Pipes::GetNamedPipeServerProcessId, Threading::*};

pub async fn start(launch: Launch, events: mpsc::Sender<Event>) -> Result<Engine> {
    launch.validate()?;
    tokio::task::spawn_blocking(super::install::start_service).await??;
    let name = pipe_name(&current_sid()?)?;
    let pipe = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match ClientOptions::new().open(&name) {
                Ok(pipe) => break Ok::<_, anyhow::Error>(pipe),
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    })
    .await
    .context("后台服务连接超时")??;
    unsafe {
        let mut pid = 0;
        check(GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut pid))?;
        super::install::verify_server(pid)?;
    }
    let mut pipe = Framed::new(pipe, LinesCodec::new_with_max_length(MAX_MESSAGE));
    pipe.send(serde_json::to_string(&Command::Start { launch })?)
        .await?;
    let line = tokio::time::timeout(Duration::from_secs(10), pipe.next())
        .await?
        .context("后台服务未启动引擎")??;
    let pid = match serde_json::from_str(&line)? {
        Reply::Started { pid } => pid,
        Reply::Error { message } => anyhow::bail!(message),
        _ => anyhow::bail!("后台服务返回无效启动确认"),
    };
    let child = Handle::new(unsafe {
        OpenProcess(
            SYNCHRONIZE_PROCESS | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        )
    })?;
    let (ipc_stop, mut ipc_task) = Engine::observe_service_ipc(pid, events.clone());
    let (stop, mut rx) = mpsc::channel::<oneshot::Sender<Result<()>>>(1);
    let task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        let mut last = tokio::time::Instant::now();
        let mut stopping = None;
        let mut stop_deadline = None;
        let reason = loop {
            tokio::select! {
                _ = interval.tick() => {
                    if child.wait(0) {
                        break "服务引擎已退出".to_owned();
                    }
                    if last.elapsed() > Duration::from_secs(5)
                        || stop_deadline.is_some_and(|t| tokio::time::Instant::now() > t)
                    {
                        break "服务响应超时".into();
                    }
                    if pipe.send(serde_json::to_string(&Command::Ping).unwrap()).await.is_err() {
                        break "服务租约连接已断开".into();
                    }
                }
                request = rx.recv(), if stopping.is_none() => {
                    let Some(request) = request else { break "应用已释放服务租约".into(); };
                    stopping = Some(request);
                    stop_deadline = Some(tokio::time::Instant::now() + Duration::from_secs(7));
                    let _ = ipc_stop.send(()).await;
                    // Flush the owned core's graceful stop before asking the
                    // supervisor to enforce job termination.
                    let _ = tokio::time::timeout(Duration::from_secs(1), &mut ipc_task).await;
                    if pipe.send(serde_json::to_string(&Command::Stop).unwrap()).await.is_err() {
                        break "服务停止连接已断开".into();
                    }
                }
                line = pipe.next() => {
                    let Some(Ok(line)) = line else { break "服务连接已断开".into(); };
                    match serde_json::from_str::<Reply>(&line) {
                        Ok(Reply::Pong) => last = tokio::time::Instant::now(),
                        Ok(Reply::Stopped) => break "stopped".into(),
                        Ok(Reply::Output { line }) => { let _ = events.try_send(Event::Output(line)); },
                        Ok(Reply::Error { message }) => break message,
                        _ => break "服务响应无效".into(),
                    }
                }
            }
        };
        drop(pipe); // EOF cancels the lease even when the stop reply is lost.
        ipc_task.abort();
        // Keep ownership and the task alive until the actual process exits.
        // A failed/timed-out service stop must not permit a second engine.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        while !child.wait(0) {
            if tokio::time::Instant::now() >= deadline {
                if let Some(reply) = stopping.take() {
                    let _ = reply.send(Err(anyhow::anyhow!("服务引擎尚未退出，禁止重复启动")));
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(250)) => {},
                Some(reply) = rx.recv() => {
                    let _ = reply.send(Err(anyhow::anyhow!("仍在等待服务引擎退出，禁止重复启动")));
                }
            }
        }
        if let Some(reply) = stopping {
            let _ = reply.send(Ok(()));
        }
        tokio::spawn(async move {
            let _ = events.send(Event::Exited(reason)).await;
        });
    });
    Ok(Engine::from_service(pid, stop, task))
}
