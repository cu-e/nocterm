use super::*;
use std::{
    io::{BufRead as _, BufReader},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
struct Bus {
    child: Child,
    address: String,
}
impl Bus {
    fn new() -> Self {
        let mut child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut address = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        Self {
            child,
            address: address.trim().into(),
        }
    }
    async fn connect(&self) -> Connection {
        zbus::connection::Builder::address(self.address.as_str())
            .unwrap()
            .build()
            .await
            .unwrap()
    }
}
impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
struct FaultingBroker {
    started: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
}
#[zbus::interface(name = "dev.nocterm.VaultBroker1")]
impl FaultingBroker {
    fn attempts_remaining(&self, _binding: &str, _token: &str) -> zbus::fdo::Result<u8> {
        if self.started.load(Ordering::SeqCst) {
            Err(zbus::fdo::Error::AccessDenied("progress denied".into()))
        } else {
            Ok(3)
        }
    }
    async fn release(&self, _binding: &str, _token: &str) -> zbus::fdo::Result<Vec<u8>> {
        self.started.store(true, Ordering::SeqCst);
        while !self.cancelled.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        Err(zbus::fdo::Error::AccessDenied("cancelled".into()))
    }
    fn cancel(&self, _binding: &str, _token: &str) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}
struct OldBroker;
#[zbus::interface(name = "dev.nocterm.VaultBroker1")]
impl OldBroker {
    fn release(&self, _binding: &str, _token: &str) -> Vec<u8> {
        vec![42; 32]
    }
}
struct CancelledBroker;
#[zbus::interface(name = "dev.nocterm.VaultBroker1")]
impl CancelledBroker {
    fn attempts_remaining(&self, _binding: &str, _token: &str) -> u8 {
        2
    }
    async fn release(&self, _binding: &str, _token: &str) -> zbus::fdo::Result<Vec<u8>> {
        tokio::time::sleep(Duration::from_millis(100)).await;
        Err(zbus::fdo::Error::AccessDenied(
            "Authentication cancelled".into(),
        ))
    }
}
#[tokio::test]
async fn failed_progress_request_cancels_inflight_broker_authentication() {
    let bus = Bus::new();
    let service = bus.connect().await;
    let client = bus.connect().await;
    let started = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::new(AtomicBool::new(false));
    service
        .object_server()
        .at(
            PATH,
            FaultingBroker {
                started: started.clone(),
                cancelled: cancelled.clone(),
            },
        )
        .await
        .unwrap();
    service.request_name(BROKER).await.unwrap();
    let proxy = Proxy::new(&client, BROKER, PATH, BROKER).await.unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        release_key(&proxy, &("binding", "token"), &DeviceCancellation::never()),
    )
    .await
    .unwrap();
    assert!(matches!(result,Err(E::Platform(error)) if error == "progress denied"));
    assert!(started.load(Ordering::SeqCst));
    assert!(cancelled.load(Ordering::SeqCst));
}
#[tokio::test]
async fn old_broker_without_progress_still_releases_authenticated_key() {
    let bus = Bus::new();
    let service = bus.connect().await;
    let client = bus.connect().await;
    service.object_server().at(PATH, OldBroker).await.unwrap();
    service.request_name(BROKER).await.unwrap();
    let proxy = Proxy::new(&client, BROKER, PATH, BROKER).await.unwrap();
    let key = release_key(&proxy, &("binding", "token"), &DeviceCancellation::never())
        .await
        .unwrap();
    assert_eq!(*key, [42; 32]);
}

#[tokio::test]
async fn remote_cancel_terminates_release_while_progress_is_still_available() {
    let bus = Bus::new();
    let service = bus.connect().await;
    let client = bus.connect().await;
    service
        .object_server()
        .at(PATH, CancelledBroker)
        .await
        .unwrap();
    service.request_name(BROKER).await.unwrap();
    let proxy = Proxy::new(&client, BROKER, PATH, BROKER).await.unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        release_key(&proxy, &("binding", "token"), &DeviceCancellation::never()),
    )
    .await
    .expect("a remote cancellation must not leave the client polling indefinitely");
    assert!(matches!(result, Err(E::Platform(error)) if error == "Authentication cancelled"));
}
