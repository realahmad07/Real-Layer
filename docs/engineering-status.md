# Engineering status

This document captures the exact engineering state of the project and the proof gate that remains before any VPN capability claim is valid.

## Objective

The present objective is narrow and measurable: prove that real packets reach the Android TUN and are successfully visible to the native Rust read loop without redesigning the application or replacing the existing architecture.

## Current state

### Proven

The project has already proven the following:

- the Android app is able to reach the VPN startup flow,
- the native library loads correctly,
- the Android `VpnService` creates a TUN device,
- the Java/Kotlin service calls the native Rust entry point,
- the JNI entry reaches Rust and the startup guard is passed,
- the fd ownership problem was fixed by duplicating the TUN descriptor before using it in Rust.

Evidence captured from the emulator included the following log sequence:

```text
RealLayerVpnService: VPN_SERVICE_START
RealLayerVpnService: VPN_TUN_ESTABLISHED fd=88
RealLayerVpnService: VPN_RUST_START fd=88
REAL_LAYER_JNI: JNI_STARTCORE_ENTERED fd=88 running=0
REAL_LAYER_JNI: JNI_STARTCORE_RUNNING_SET fd=88
```

This confirms the relevant startup boundary is reached and is not blocked before the native path begins.

### Not yet proven

The project has not yet proven the following:

- real phone or emulator traffic is entering the TUN,
- the Rust read loop is receiving packet bytes with valid non-zero lengths,
- the traffic reaches any subsequent packet/data-plane stage,
- the loop remains stable across repeated connects.

Because of this, the project can claim only a successful native startup proof, not a VPN functionality claim.

## Proof boundary

The current proof boundary is:

```text
Android VpnService
    ↓
TUN file descriptor
    ↓
JNI bridge
    ↓
Rust read loop
    ↓
packet capture / packet log proof
```

A valid functional claim requires a packet to be observed at the bottom of that pipeline.

## Completed work summary

### 1. Toolchain and build repair

The Windows Android linker issue was resolved by building with the correct target-specific linker. The native library was rebuilt and repackaged into the app for Android execution.

### 2. Android app startup path

The app successfully launched the VPN flow and reached the Java/Kotlin service. The tunnel was established and the service called the native Rust function that starts the client runtime.

### 3. Descriptor ownership fix

The earlier fatal issue was caused when Rust owned the same file descriptor that Android still needed to manage. The fix was to duplicate the original TUN fd and keep the duplicate as the Rust-owned descriptor, preserving the original Android-owned fd.

### 4. Runtime instrumentation

The native bridge was instrumented to log entry and startup markers so the exact debug boundary is visible in `logcat`.

## Remaining proof work

The remaining work is intentionally small and specific:

1. trigger the VPN flow,
2. generate one real network request from the emulator/device,
3. confirm a non-zero packet length on the TUN read loop,
4. capture the packet log line as proof,
5. repeat for three clean cycles before any broader claim is accepted.

## Guardrails

- no UI redesign,
- no replacement of the working networking architecture,
- no relay or forwarding claim before packet proof,
- no claim of Internet VPN behavior before TUN ingress is proven,
- no synthetic packet-only proof accepted as a substitute for real traffic.

## Current status summary

The project is in a validated startup state and a packet-proof gate.

Status:

- startup path: proven,
- native TUN entry: proven,
- descriptor ownership bug: fixed,
- real packet ingress: pending,
- VPN functionality claim: blocked pending proof.
