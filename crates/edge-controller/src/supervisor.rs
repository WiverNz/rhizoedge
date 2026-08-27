//! Named task supervision and bounded graceful shutdown.
#![allow(missing_docs)]
use crate::metrics::Metrics;
use std::future::Future;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinSet;

/// Process task owner. Any unexpected task return is fatal.
pub struct Supervisor {
    tasks: JoinSet<(&'static str, Result<(), String>)>,
    shutdown: watch::Sender<bool>,
    metrics: Metrics,
    timeout: Duration,
}
impl Supervisor {
    pub fn new(metrics: Metrics, timeout: Duration) -> Self {
        let (shutdown, _) = watch::channel(false);
        Self {
            tasks: JoinSet::new(),
            shutdown,
            metrics,
            timeout,
        }
    }
    pub fn shutdown_receiver(&self) -> watch::Receiver<bool> {
        self.shutdown.subscribe()
    }
    pub fn spawn<F>(&mut self, name: &'static str, f: F)
    where
        F: Future<Output = Result<(), String>> + Send + 'static,
    {
        self.tasks.spawn(async move {
            let result = match tokio::spawn(f).await {
                Ok(value) => value,
                Err(join) => Err(format!("panic:{join}")),
            };
            (name, result)
        });
    }
    pub async fn run(mut self) -> Result<(), String> {
        let requested = self.shutdown.subscribe();
        tokio::select! {
         _=shutdown_signal(requested)=>{let _=self.shutdown.send(true);let drain=async{while let Some(result)=self.tasks.join_next().await{match result{Ok((_name,Ok(())))=>{},Ok((name,Err(e)))=>return task_error(&self.metrics,name,e),Err(e)=>return Err(e.to_string())}}Ok(())};match tokio::time::timeout(self.timeout,drain).await{Ok(result)=>result,Err(_)=>{tracing::warn!("shutdown timeout reached; aborting remaining tasks at transaction boundary");Ok(())}}}
         result=self.tasks.join_next()=>{match result{Some(Ok((name,Err(e))))=>task_error(&self.metrics,name,e),Some(Ok((name,Ok(()))))=>Err(format!("task {name} exited unexpectedly")),Some(Err(join))=>Err(format!("supervisor join failure: {join}")),None=>Err("no supervised tasks".into())}}
        }
    }
}
fn task_error(metrics: &Metrics, name: &'static str, error: String) -> Result<(), String> {
    if error.starts_with("panic:") {
        metrics.task_panics.with_label_values(&[name]).inc();
        tracing::error!(task=name,error=%error,"supervised task panicked");
    }
    Err(format!("task {name}: {error}"))
}
async fn shutdown_signal(mut requested: watch::Receiver<bool>) {
    #[cfg(unix)]
    {
        if let Ok(mut term) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! {_=term.recv()=>{},_=tokio::signal::ctrl_c()=>{},_=requested.wait_for(|v|*v)=>{}}
            return;
        }
    }
    tokio::select! {_=tokio::signal::ctrl_c()=>{},_=requested.wait_for(|v|*v)=>{}}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn panic_is_failure() {
        let m = Metrics::new().unwrap();
        let before = m.task_panics.with_label_values(&["pipeline"]).get();
        let mut s = Supervisor::new(m.clone(), Duration::from_millis(50));
        s.spawn("pipeline", async { panic!("forced") });
        assert!(s.run().await.unwrap_err().contains("panic"));
        assert_eq!(
            m.task_panics.with_label_values(&["pipeline"]).get(),
            before + 1
        );
    }

    #[tokio::test]
    async fn requested_shutdown_drains_cooperative_task() {
        let mut s = Supervisor::new(Metrics::new().unwrap(), Duration::from_millis(100));
        let stop = s.shutdown.clone();
        let mut worker_stop = s.shutdown_receiver();
        s.spawn("pipeline", async move {
            worker_stop.wait_for(|value| *value).await.unwrap();
            Ok(())
        });
        let running = tokio::spawn(s.run());
        tokio::task::yield_now().await;
        stop.send(true).unwrap();
        assert_eq!(running.await.unwrap(), Ok(()));
    }

    #[tokio::test]
    async fn shutdown_timeout_bounds_a_hung_task() {
        let mut s = Supervisor::new(Metrics::new().unwrap(), Duration::from_millis(20));
        let stop = s.shutdown.clone();
        s.spawn("hung", std::future::pending());
        let running = tokio::spawn(s.run());
        tokio::task::yield_now().await;
        stop.send(true).unwrap();
        assert_eq!(running.await.unwrap(), Ok(()));
    }
}
