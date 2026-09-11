#[cfg(target_os = "android")]
use jni::objects::JClass;
#[cfg(target_os = "android")]
use jni::JNIEnv;
#[cfg(target_os = "android")]
use std::thread;
#[cfg(target_os = "android")]
use std::os::unix::io::FromRawFd;
#[cfg(target_os = "android")]
use std::fs::File;
#[cfg(target_os = "android")]
use std::io::{Read, Write};
#[cfg(target_os = "android")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "android")]
use std::sync::Arc;

#[cfg(target_os = "android")]
static RUNNING: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_com_example_magicblock_1app_RealLayerVpnService_startCore(
    mut _env: JNIEnv,
    _class: JClass,
    fd: i32,
) {
    if RUNNING.load(Ordering::SeqCst) {
        println!("Native VPN Core is already running.");
        return;
    }
    RUNNING.store(true, Ordering::SeqCst);
    println!("Native VPN Core started with TUN fd: {}", fd);

    thread::spawn(move || {
        let fd_read = fd;
        let fd_write = unsafe { libc::dup(fd) };
        if fd_write < 0 {
            println!("Failed to dup TUN fd");
            return;
        }

        let mut read_file = unsafe { std::fs::File::from_raw_fd(fd_read) };
        let mut write_file = unsafe { std::fs::File::from_raw_fd(fd_write) };

        let (tun_tx_in, tun_rx_in) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let (tun_tx_out, mut tun_rx_out) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();

        // Read thread
        thread::spawn(move || {
            let mut buf = vec![0u8; 65535];
            while RUNNING.load(Ordering::SeqCst) {
                match read_file.read(&mut buf) {
                    Ok(n) if n > 0 => {
                        let _ = tun_tx_in.send(buf[..n].to_vec());
                    }
                    Ok(_) => thread::sleep(std::time::Duration::from_millis(10)),
                    Err(_) => break,
                }
            }
        });

        // Write thread
        let write_running = Arc::new(AtomicBool::new(true));
        let write_running_clone = write_running.clone();
        thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                while let Some(packet) = tun_rx_out.recv().await {
                    if !write_running_clone.load(Ordering::SeqCst) { break; }
                    let _ = write_file.write_all(&packet);
                }
            });
        });
        
        // Inject node configuration for Android environment.
        // 10.0.2.2 is the Android emulator's alias for the host loopback.
        // These values must match the running relay on the host.
        unsafe {
            std::env::set_var("GHOST_NODE_ID", "android-client-1");
            std::env::set_var("GHOST_LISTEN_ADDRESS", "/ip4/0.0.0.0/udp/0/quic-v1");
            std::env::set_var("GHOST_ADVERTISED_ADDRESS", "/ip4/10.0.2.2/udp/0/quic-v1");
            std::env::set_var("GHOST_NETWORK_ENVIRONMENT", "development");
            std::env::set_var("GHOST_RELAY_ROLE", "client");
            std::env::set_var("GHOST_LOG_LEVEL", "info");
            std::env::set_var("GHOST_IDENTITY_PATH", "/data/data/com.example.magicblock_app/files/ghost-layer.key");
            std::env::set_var(
                "GHOST_BOOTSTRAP_PEERS",
                "/ip4/10.0.2.2/udp/7000/quic-v1/p2p/12D3KooWF9mfD7d2VEabciAStgdfiSA1pXi5rcyYmpSne2aXyDN6",
            );
            std::env::set_var("GHOST_CONNECTION_TIMEOUT_SECS", "30");
            std::env::set_var("GHOST_HEARTBEAT_INTERVAL_SECS", "10");
            std::env::set_var("GHOST_HEARTBEAT_FRESHNESS_SECS", "60");
            std::env::set_var("GHOST_DEGRADED_LATENCY_MS", "500");
            std::env::set_var("GHOST_SOFTWARE_VERSION", "0.1.0");
            std::env::set_var("GHOST_PROTOCOL_VERSION", "1.0");
            std::env::set_var("GHOST_DISCOVERY_REQUIRED_TRANSPORT", "quic");
            std::env::set_var("GHOST_DISCOVERY_REQUIRED_CAPABILITIES", "relay");
            std::env::set_var("GHOST_ROUTE_MODE", "one-hop");
        }

        let rt = tokio::runtime::Runtime::new().unwrap();
        if let Err(e) = rt.block_on(engine::run_client(Some(tun_rx_in), Some(tun_tx_out))) {
            println!("Engine failed: {}", e);
        }
        
        write_running.store(false, Ordering::SeqCst);
        println!("Native VPN Core terminated.");
    });
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_com_example_magicblock_1app_RealLayerVpnService_stopCore(
    mut _env: JNIEnv,
    _class: JClass,
) {
    println!("Native VPN Core stopping...");
    RUNNING.store(false, Ordering::SeqCst);
}

pub mod engine;

#[cfg(target_os = "android")]
pub mod android_tun;
