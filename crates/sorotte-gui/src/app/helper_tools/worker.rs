use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

/// The GUI owns cancellation; a dropped receiver never leaves a blocked progress sender.
pub(in crate::app) struct HelperWorker<E> {
    pub(in crate::app) root: Option<PathBuf>,
    pub(in crate::app) rx: mpsc::Receiver<E>,
    cancel: Arc<AtomicBool>,
}

impl<E: Send + 'static> HelperWorker<E> {
    pub(in crate::app) fn spawn(
        name: &str,
        root: Option<PathBuf>,
        run: impl FnOnce(&AtomicBool, mpsc::Sender<E>) + Send + 'static,
    ) -> Result<Self, String> {
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || run(&worker_cancel, tx))
            .map_err(|error| format!("Could not start helper worker: {error}"))?;
        Ok(Self { root, rx, cancel })
    }
}

impl<E> Drop for HelperWorker<E> {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
