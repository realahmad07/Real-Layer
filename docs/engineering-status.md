# Engineering status

This document captures the real engineering state of the project and the exact proof gate that remains before any VPN capability claim is valid.

## Objective

The current objective is intentionally narrow and measurable: prove that real packets reach the Android TUN and are visible to the native Rust read loop without redesigning the app or replacing the existing architecture.

## Current state

### Proven

The project has already shown the following:

- the Android app can enter the VPN startup flow,
- the native library loads correctly,
- the Android `VpnService` creates a TUN,
- the Java/Kotlin service invokes the native Rust entry,
- the JNI bridge reaches Rust and passes the startup guard,
- the fd ownership problem was fixed by duplicating the original TUN descriptor before Rust takes ownership.

The observed emulator log sequence is:

```text
RealLayerVpnService: VPN_SERVICE_START
RealLayerVpnService: VPN_TUN_ESTABLISHED fd=88
RealLayerVpnService: VPN_RUST_START fd=88
REAL_LAYER_JNI: JNI_STARTCORE_ENTERED fd=88 running=0
REAL_LAYER_JNI: JNI_STARTCORE_RUNNING_SET fd=88
```

This confirms the startup boundary is reached. It does not yet confirm packet ingress.

### Not yet proven

The remaining unproven items are the actual proof gate:

- real app or emulator traffic enters the TUN,
- the Rust read loop receives packet bytes with non-zero length,
- the traffic reaches the next packet-processing boundary,
- the path remains stable across repeated clean runs.

Because those conditions are still unproven, the project remains in a startup-proven, packet-proof-pending state.

---

## Proof boundary

The critical boundary is:

```text
Android VpnService
    ↓
TUN file descriptor
    ↓
JNI bridge
    ↓
Rust read loop
    ↓
packet log proof
```

A valid functional claim requires a packet to be observed at the bottom of this pipeline.

---

## Completed work summary

### 1. Toolchain and build repair

The Android toolchain issue was corrected by using the correct target-specific linker. The native library was rebuilt and repackaged correctly for Android execution.

### 2. Android startup path

The app successfully launches the VPN flow and reaches the Kotlin service. The tunnel is created and the native entry point is invoked.

### 3. Descriptor ownership fix

The earlier crash was caused by a file descriptor ownership bug: Rust was attempting to own the same descriptor Android still needed to manage. The fix was to duplicate the original TUN fd and keep the duplicate as the Rust-owned descriptor.

### 4. Runtime instrumentation

The JNI bridge was instrumented to log startup progress and the exact boundary at which the TUN path enters Rust. These logs are the current evidentiary boundary for the project.

---

## Remaining proof work

The remaining work is intentionally small and specific:

1. trigger the VPN flow,
2. generate one real network request from the emulator/device,
3. confirm a non-zero packet size in the TUN read loop,
4. capture the packet log line as evidence,
5. repeat the sequence for three clean validation cycles before making a broader claim.

---

## Guardrails

- no UI redesign,
- no replacement of the working architecture,
- no relay claim before packet-proof,
- no Internet VPN claim before TUN ingress is proven,
- no synthetic or non-device traffic substitution for the proof gate.

---

## Current status summary

Status:

- startup path: proven,
- native TUN entry: proven,
- descriptor ownership bug: fixed,
- real packet ingress: pending,
- VPN functionality claim: blocked until packet proof is captured.
