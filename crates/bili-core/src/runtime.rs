use anyhow::{Context, Result, anyhow};
use async_channel::{Receiver, Sender};
use futures::FutureExt;
use std::{
    future::Future,
    panic::AssertUnwindSafe,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

type Job = Pin<Box<dyn Future<Output = ()> + Send>>;

#[derive(Clone)]
pub struct CoreRuntime {
    sender: Sender<Job>,
    stopped: Arc<AtomicBool>,
    finished: Receiver<()>,
}

impl CoreRuntime {
    pub fn start() -> Result<Self> {
        let (sender, receiver) = async_channel::bounded::<Job>(128);
        let (done_tx, finished) = async_channel::bounded(1);
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("bili-tokio-runtime".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .thread_name("bili-worker")
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.to_string()));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                runtime.block_on(async move {
                    let mut tasks = tokio::task::JoinSet::new();
                    loop {
                        tokio::select! {
                            job = receiver.recv(), if tasks.len() < 32 => match job {
                                Ok(job) => { tasks.spawn(job); }
                                Err(_) => break,
                            },
                            _ = tasks.join_next(), if !tasks.is_empty() => {},
                        }
                    }
                    while tasks.join_next().await.is_some() {}
                });
                drop(runtime);
                let _ = done_tx.send_blocking(());
            })
            .context("無法啟動核心背景執行緒")?;
        ready_rx
            .recv()
            .context("核心 Runtime 未回應")?
            .map_err(|message| anyhow!(message))?;
        Ok(Self {
            sender,
            stopped: Arc::new(AtomicBool::new(false)),
            finished,
        })
    }

    pub fn submit<T: Send + 'static>(
        &self,
        future: impl Future<Output = Result<T>> + Send + 'static,
    ) -> Result<Receiver<Result<T>>> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(anyhow!("核心服務正在退出"));
        }
        let (reply, result) = async_channel::bounded(1);
        let job = Box::pin(async move {
            let result = AssertUnwindSafe(future)
                .catch_unwind()
                .await
                .unwrap_or_else(|_| Err(anyhow!("核心背景工作意外中止")));
            let _ = reply.send(result).await;
        });
        self.sender
            .try_send(job)
            .map_err(|_| anyhow!("核心工作佇列已滿，請稍後重試"))?;
        Ok(result)
    }

    pub fn submit_cancellable<T: Send + 'static>(
        &self,
        cancel: tokio_util::sync::CancellationToken,
        future: impl Future<Output = Result<T>> + Send + 'static,
    ) -> Result<Receiver<Result<T>>> {
        self.submit(async move {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => Err(anyhow!("工作已取消")),
                result = future => result,
            }
        })
    }

    pub async fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        self.sender.close();
        let _ = self.finished.recv().await;
    }
}
