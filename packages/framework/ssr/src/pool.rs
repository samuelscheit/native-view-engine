//! Dedicated OS worker pool running single-threaded Tokio `LocalSet` runtimes for `!Send`
//! Dioxus streaming SSR requests with round-robin dispatch and non-panicking backpressure.

use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use http::Response;
use nve_dioxus::DioxusSurface;
use nve_http_hyper::HttpBody;
use nve_query_dioxus::ErasedQueryClient;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, oneshot};

use crate::options::{SsrLocalPoolConfig, SsrPageOptions, StreamPlan};
use crate::render::execute_render_streaming;
use crate::SsrError;

type LocalBoxFuture = Pin<Box<dyn Future<Output = ()> + 'static>>;
type SsrJob = Box<dyn FnOnce() -> LocalBoxFuture + Send + 'static>;

struct SsrLocalPoolInner {
    workers: Vec<mpsc::Sender<SsrJob>>,
    next_worker: AtomicUsize,
    queue_capacity_per_worker: usize,
}

/// Cloneable handle to a fixed pool of dedicated OS worker threads running single-threaded Tokio
/// `LocalSet` runtimes for `!Send` streaming SSR tasks.
///
/// Dropping the last [`SsrLocalPool`] clone closes every worker channel immediately without
/// blocking; each worker finishes draining any already-dispatched streaming responses and exits
/// cleanly on its own thread.
#[derive(Clone)]
pub struct SsrLocalPool {
    inner: Arc<SsrLocalPoolInner>,
}

impl core::fmt::Debug for SsrLocalPool {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SsrLocalPool")
            .field("worker_count", &self.inner.workers.len())
            .field(
                "queue_capacity_per_worker",
                &self.inner.queue_capacity_per_worker,
            )
            .finish()
    }
}

impl SsrLocalPool {
    /// Creates an [`SsrLocalPool`] with [`SsrLocalPoolConfig::default`]
    /// (`available_parallelism().clamp(1, 16)` workers).
    pub fn new() -> Result<Self, SsrError> {
        Self::with_config(SsrLocalPoolConfig::default())
    }

    /// Creates an [`SsrLocalPool`] with explicit worker count and per-worker bounded queue depth.
    pub fn with_config(config: SsrLocalPoolConfig) -> Result<Self, SsrError> {
        let worker_count = config.worker_count.max(1);
        let queue_capacity = config.queue_capacity_per_worker.max(1);
        let mut workers = Vec::with_capacity(worker_count);

        for idx in 0..worker_count {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|err| SsrError::PoolInit(format!("worker {idx} runtime: {err}")))?;
            let (tx, mut rx) = mpsc::channel::<SsrJob>(queue_capacity);

            std::thread::Builder::new()
                .name(format!("nve-ssr-{idx}"))
                .spawn(move || {
                    let local = tokio::task::LocalSet::new();
                    rt.block_on(async move {
                        local
                            .run_until(async move {
                                while let Some(job) = rx.recv().await {
                                    tokio::task::spawn_local(job());
                                }
                            })
                            .await;
                        // Drain already-dispatched streaming tasks cleanly before thread exit.
                        local.await;
                    });
                })
                .map_err(|err| SsrError::PoolInit(format!("worker {idx} thread: {err}")))?;

            workers.push(tx);
        }

        Ok(Self {
            inner: Arc::new(SsrLocalPoolInner {
                workers,
                next_worker: AtomicUsize::new(0),
                queue_capacity_per_worker: queue_capacity,
            }),
        })
    }

    /// Number of dedicated `LocalSet` worker threads in this pool.
    #[inline]
    pub fn worker_count(&self) -> usize {
        self.inner.workers.len()
    }

    /// Bounded pending job queue capacity configured per worker thread.
    #[inline]
    pub fn queue_capacity_per_worker(&self) -> usize {
        self.inner.queue_capacity_per_worker
    }

    /// Dispatches a job factory onto the next available worker in round-robin order without
    /// blocking or panicking. Scans remaining workers if the chosen slot is full so one busy
    /// worker never causes premature backpressure while sibling workers have capacity.
    fn dispatch(&self, mut job: SsrJob) -> Result<(), SsrError> {
        let workers = &self.inner.workers;
        let len = workers.len();
        let start = self.inner.next_worker.fetch_add(1, Ordering::Relaxed) % len;
        let mut saw_full = false;

        for offset in 0..len {
            let idx = (start + offset) % len;
            match workers[idx].try_send(job) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Full(returned)) => {
                    saw_full = true;
                    job = returned;
                }
                Err(TrySendError::Closed(returned)) => {
                    job = returned;
                }
            }
        }

        if saw_full {
            Err(SsrError::PoolOverloaded)
        } else {
            Err(SsrError::PoolClosed)
        }
    }

    /// Renders a Dioxus application on a dedicated `LocalSet` worker, executing Frame 1 and
    /// Declarative Shadow DOM placement hygiene BEFORE emitting HTTP headers.
    ///
    /// - **Adaptive fast path**: When zero `<QuerySuspense>` boundaries suspend, resolves to a
    ///   buffered `Response<HttpBody::Full>` Light-DOM HTML5 document with zero shadow root.
    /// - **Streaming path**: When one or more slots suspend, resolves immediately to
    ///   `Response<HttpBody::Stream>` carrying the DSD shell + `seq=0` query payload + optional
    ///   async boot script while the `LocalSet` worker streams out-of-order `[NVEQ + ack + slot]`
    ///   deltas until completion, `plan.total_deadline`, or client disconnect.
    pub async fn render_streaming<F>(
        &self,
        page: SsrPageOptions,
        plan: StreamPlan,
        client: ErasedQueryClient,
        build_surface: F,
    ) -> Result<Response<HttpBody>, SsrError>
    where
        F: FnOnce(ErasedQueryClient) -> DioxusSurface + Send + 'static,
    {
        let (headers_tx, headers_rx) = oneshot::channel::<Result<Response<HttpBody>, SsrError>>();

        let job: SsrJob = Box::new(move || {
            Box::pin(async move {
                execute_render_streaming(page, plan, client, build_surface, headers_tx).await;
            })
        });

        self.dispatch(job)?;

        headers_rx.await.map_err(|_| SsrError::PoolClosed)?
    }
}
