use tokio::sync::watch;

pub(in crate::manager) async fn await_activation(mut activation: watch::Receiver<bool>) -> bool {
    while !*activation.borrow_and_update() {
        if activation.changed().await.is_err() {
            return false;
        }
    }
    true
}
