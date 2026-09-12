# Real Layer

Real Layer is a proof-driven networking and relay prototype focused on secure routing, bounded forwarding, Android TUN integration, and controlled client-to-relay communication. The repository currently represents a technical foundation for private routing experiments rather than a public VPN or unrestricted Internet access layer.

## Hackathon Demo Phase

We are currently preparing for the Real Layer hackathon submission. The priority of this phase is validating the core Android application, secure backend lifecycle, and L4 app-level tunneling foundation. 

> **Important Constraint Notice:** Blockchain functionality (Solana, Anchor, MagicBlock) and DePIN/token incentives are **intentionally parked** during this phase to focus strictly on validating the core cryptographic networking foundation. Unrestricted system-wide OS routing and Windows packaging are also parked in favor of a secure, allowlisted app-level tunnel on Android.

## Architecture

The following diagram illustrates the active data path and component status for the mobile Android client connecting to the Ghost Layer network.

`mermaid
flowchart TD
    %% Mobile Layer
    UI[Flutter UI<br/>*State Monitoring*] -->|HTTP 127.0.0.1:8082| RustClient
    VpnSvc[Android VpnService<br/>*TUN Interface*] -->|Raw IP Packets<br/>Split Tunnel| RustJNI
    
    %% Rust Native Boundary
    subgraph Mobile Device
    RustJNI[Rust JNI Layer<br/>*FD Duplication*] --> RustClient[Rust Client Engine<br/>*ghost-layer-client*]
    RustClient -->|L4 Extraction| Encrypt[Encrypted Session]
    end

    %% Network Transport
    Encrypt -->|QUIC / libp2p| Entry[Entry Relay<br/>*ghost-layer-relay*]
    Entry -->|Secure Forwarding| Exit[Exit Relay<br/>*ghost-layer-relay*]
    
    %% Exit Boundary
    Exit -->|L4 UDP/TCP Adapter<br/>*Destination Allowlist*| Web[Real Internet<br/>e.g., 8.8.8.8:53]

    %% Styles and Status Labels
    classDef implemented fill:#4caf50,stroke:#2e7d32,stroke-width:2px,color:white;
    classDef partial fill:#ff9800,stroke:#ef6c00,stroke-width:2px,color:white;
    classDef parked fill:#9e9e9e,stroke:#616161,stroke-width:2px,color:white;

    class UI,RustClient,RustJNI,Encrypt,Entry,Exit implemented
    class VpnSvc,Web partial
`

### Component Status Key:
* **[IMPLEMENTED]** (Green): Flutter UI, JNI boundaries, Rust QUIC Client, Encrypted Sessions, Entry/Exit Relays, L4 Forwarding Adapters.
* **[IN PROGRESS]** (Orange): Android VpnService (Split tunneling active for DNS demonstration), Real Internet (Restricted to allowlisted testing endpoints to prevent OS routing leaks).
* **[NOT YET VALIDATED / PARKED]**: Unrestricted System-wide OS Routing, Public IP Deployment, Solana/Anchor Smart Contracts.

## Current Project Status

### **Implemented & Validated**
* Rust/libp2p QUIC networking core.
* Two-hop routing (Client ? Entry ? Exit) with preserved Exit Multiaddresses.
* Encrypted session establishment (Client ? Entry, Entry ? Exit).
* TCP/UDP/DNS protocol extraction and runtime paths.
* Exit destination allowlisting and safety validation.
* Flutter/Dart mobile application UI.
* Android JNI integration (Native thread lifecycle, TUN file descriptor duplication).
* True backend state monitoring (Disconnected, Connecting, Protected, Error) bridging Flutter and Rust natively.
* L4 App-level tunnel foundation (Android intercepting 8.8.8.8 for verifiable end-to-end DNS forwarding).

### **Left for Final Demo**
* Remote network preparation with Public IPs.
* Final remote connection and traffic verification.
* End-to-end network failure and reconnection resilience testing.

### **Parked for Current Phase**
* Windows production release packaging and ACL hardening.
* Azure/Public network deployment.
* Solana, Anchor, MagicBlock blockchain integration.
* DePIN tokenomics and incentive mechanisms.
* Full L3/System-wide VPN routing without allowlist restrictions.

## Running the Application (Local Development)

The Android emulator requires access to the host's loopback interface. 

1. Start the Entry Relay on the host machine.
2. Start the Exit Relay on the host machine.
3. Build and launch the Flutter Android application:
   \\\ash
   cd real_layer_app
   flutter run -d android
   \\\
4. Tap **Connect** to initialize the Native VpnService, spawn the Rust client, establish QUIC connections, and monitor the true cryptographic state.
