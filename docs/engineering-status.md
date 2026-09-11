# Engineering status

This document captures the current phase of the project and the exact gate that must be satisfied before any VPN claim is made.

## Objective

The active objective is to prove that real packets enter the Android TUN interface and are read by the native Rust layer, without redesigning the application or bypassing the established network architecture.

## Proof boundary

The critical boundary is:

```text
Android VpnService
    ↓
TUN file descriptor
    ↓
Rust JNI bridge
    ↓
TUN read loop
    ↓
packet log + proof capture
```

Only after real traffic is observed at this boundary can any claim be made about VPN behavior or packet forwarding.

## Completed work

### 1. Native build repaired

The initial blocker was the Android linker path. The project was rebuilt using the target-specific linker from the Android NDK:

- `aarch64-linux-android30-clang.cmd`
- proper `CC_*` and `CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER` environment values

This resolved the Windows-specific toolchain failure and produced a valid release build for the ARM64 target.

### 2. Android app rebuilt and installed

The Rust native library was copied into the Android app's JNI directory and the APK was rebuilt and installed to the emulator. This included the required native library artifact for the app runtime.

### 3. TUN startup path verified

The service reached the expected Android logging path:

- `VPN_SERVICE_START`
- `VPN_TUN_ESTABLISHED fd=<n>`
- `VPN_RUST_START fd=<n>`

This confirmed the VPN permission flow and TUN creation path were working as expected.

### 4. FD ownership root cause fixed

The fatal issue was a descriptor ownership problem. Android owns the original `ParcelFileDescriptor` backing the TUN. Rust previously converted the raw descriptor into a Rust-owned `File` without duplication, which triggered the Android fdsan close error.

The fix was to duplicate the TUN fd first using `dup()` and then convert the duplicated descriptor into Rust-owned file handles. This preserves Android's ownership of the original descriptor while allowing the TUN read loop to operate safely.

## Remaining proof work

The following is still required before any software claim is accepted:

### 1. Real packet ingress proof

Generate genuine traffic from the emulator or Android device while the VPN is running and confirm logs such as:

```text
VPN_TUN_PACKET_RX length=<actual number> direction=INBOUND
TUN_PACKET INBOUND len=... protocol=... src=... dst=...
```

### 2. Three-cycle validation

The proof must pass exactly three independent cycles:

1. Connect
2. Generate real traffic
3. Observe TUN packet RX
4. Disconnect and reconnect
5. Repeat again

The system must show stable behavior across all three cycles.

### 3. Stability review

The app must remain stable across reconnects without crashing, without descriptor misuse, and without losing the actual TUN read loop.

## Explicit non-goals

- UI redesign
- replacement of the existing Rust networking architecture
- relay forwarding claims before packet proof
- claiming "VPN functionality" without verified packet ingress

## Current status summary

The project has successfully passed the build and installation gate and resolved the fd ownership bug. It is now at the final proof gate: real traffic must be observed on the Android TUN before the project can legitimately claim VPN-level behavior.

No forwarding, relay, or exit-node claim should be made until this proof is captured and repeated successfully.