import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:http/http.dart' as http;

enum BackendRuntimeConnectionState {
  disconnected,
  connecting,
  connected,
  disconnecting,
  error,
}

class BackendConnectionStatus {
  const BackendConnectionStatus({
    required this.alive,
    required this.ready,
    required this.health,
    required this.latencyMs,
    required this.status,
    this.message,
  });

  static void trace(String message) {
    final stamp = DateTime.now().toIso8601String();
    debugPrint('[REAL_LAYER_TRACE] $stamp $message');
  }

  factory BackendConnectionStatus.empty() => const BackendConnectionStatus(
    alive: false,
    ready: false,
    health: 'unavailable',
    latencyMs: 0,
    status: 'disconnected',
  );

  factory BackendConnectionStatus.fromJson(Map<String, dynamic> json) {
    final rawHealth = json['health'];
    final rawStatus = json['status'];
    final latency = json['latency_ms'] ?? json['latencyMs'] ?? 0;

    final ready = json['ready'] == true;
    final healthValue = rawHealth is String ? rawHealth.toLowerCase() : '';
    final healthy = healthValue == 'healthy' || ready;
    final alive = json['alive'] == true || ready || healthy;

    return BackendConnectionStatus(
      alive: alive,
      ready: ready,
      health: rawHealth is String
          ? rawHealth
          : (ready || healthy)
          ? 'healthy'
          : 'unavailable',
      latencyMs: latency is num
          ? latency.toInt()
          : (latency is String ? int.tryParse(latency) ?? 0 : 0),
      status: rawStatus is String
          ? rawStatus
          : (ready || healthy)
          ? 'connected'
          : alive
          ? 'disconnected'
          : 'unavailable',
      message: json['message'] is String ? json['message'] as String : null,
    );
  }

  final bool alive;
  final bool ready;
  final String health;
  final int latencyMs;
  final String status;
  final String? message;

  bool get healthy => health.toLowerCase() == 'healthy';

  BackendRuntimeConnectionState get uiState {
    final normalizedStatus = status.trim().toLowerCase();

    BackendRuntimeConnectionState resolved;
    if (normalizedStatus == 'disconnecting' ||
        normalizedStatus.startsWith('disconnecting ') ||
        normalizedStatus.endsWith(' disconnecting')) {
      resolved = BackendRuntimeConnectionState.disconnecting;
    } else if (normalizedStatus == 'disconnected' ||
        normalizedStatus.startsWith('disconnected ') ||
        normalizedStatus.endsWith(' disconnected')) {
      resolved = BackendRuntimeConnectionState.disconnected;
    } else if (normalizedStatus == 'connecting' ||
        normalizedStatus.startsWith('connecting ') ||
        normalizedStatus.endsWith(' connecting')) {
      resolved = BackendRuntimeConnectionState.connecting;
    } else if (ready || healthy || alive) {
      resolved = BackendRuntimeConnectionState.connected;
    } else if (normalizedStatus.contains('error')) {
      resolved = BackendRuntimeConnectionState.error;
    } else {
      resolved = BackendRuntimeConnectionState.disconnected;
    }

    trace(
      'uiState status=$status alive=$alive ready=$ready healthy=$healthy resolved=${resolved.name}',
    );
    return resolved;
  }

  String get statusLabel {
    final normalizedStatus = status.trim().toLowerCase();

    if (normalizedStatus == 'disconnecting' ||
        normalizedStatus.startsWith('disconnecting ') ||
        normalizedStatus.endsWith(' disconnecting')) {
      return 'Disconnecting';
    }
    if (normalizedStatus == 'disconnected' ||
        normalizedStatus.startsWith('disconnected ') ||
        normalizedStatus.endsWith(' disconnected')) {
      return 'Disconnected';
    }
    if (normalizedStatus == 'connecting' ||
        normalizedStatus.startsWith('connecting ') ||
        normalizedStatus.endsWith(' connecting')) {
      return 'Connecting';
    }
    if (ready || healthy || alive) {
      return 'Protected';
    }
    if (normalizedStatus.contains('error')) {
      return 'Error';
    }
    if (!alive) {
      return 'Disconnected';
    }
    return status.isNotEmpty ? status : 'Unavailable';
  }
}

class RealLayerBackendBridge {
  RealLayerBackendBridge({this.baseUrl, this.repoRoot, this.healthPort = 8081});

  final String? baseUrl;
  final String? repoRoot;
  final int healthPort;

  void trace(String message) {
    BackendConnectionStatus.trace(message);
  }

  Process? _process;
  bool _connecting = false;

  String get resolvedBaseUrl {
    if (baseUrl != null && baseUrl!.trim().isNotEmpty) {
      return baseUrl!.trim();
    }
    if (Platform.isAndroid) {
      return 'http://10.0.2.2:$healthPort';
    }
    return 'http://127.0.0.1:$healthPort';
  }

  String get workingDirectory {
    if (repoRoot != null && repoRoot!.trim().isNotEmpty) {
      return repoRoot!.trim();
    }
    final current = Directory.current.path;
    if (current.contains('magicblock_app')) {
      return current.replaceAll(RegExp(r'\\magicblock_app$'), '');
    }
    return current;
  }

  Future<BackendConnectionStatus> fetchStatus() async {
    final endpoints = <String>[
      '$resolvedBaseUrl/health',
      '$resolvedBaseUrl/ready',
      '$resolvedBaseUrl/live',
    ];

    trace(
      'fetchStatus start baseUrl=$resolvedBaseUrl endpoints=${endpoints.join(',')}',
    );

    for (final endpoint in endpoints) {
      try {
        trace('fetchStatus GET $endpoint');
        final response = await http
            .get(Uri.parse(endpoint))
            .timeout(const Duration(seconds: 3));
        trace(
          'fetchStatus response $endpoint status=${response.statusCode} body=${response.body}',
        );
        final payload = _decodePayload(response.body);
        if (payload != null) {
          final status = BackendConnectionStatus.fromJson(payload);
          trace(
            'fetchStatus parsed endpoint=$endpoint alive=${status.alive} ready=${status.ready} health=${status.health} status=${status.status} uiState=${status.uiState.name}',
          );
          if (status.alive || status.ready || status.health != 'unavailable') {
            return status;
          }
        }
      } catch (error) {
        trace('fetchStatus exception endpoint=$endpoint error=$error');
      }
    }

    trace('fetchStatus fallback empty');
    return BackendConnectionStatus.empty();
  }

  Future<BackendConnectionStatus> connect() async {
    if (_connecting) {
      trace('connect rejected: already in progress');
      throw StateError('connection already in progress');
    }

    trace('BACKEND_BRIDGE_CONNECT_START');
    trace('connect start');
    _connecting = true;
    try {
      if (!Platform.isWindows && !Platform.isMacOS && !Platform.isLinux) {
        trace('connect bypass fetchStatus on mobile');
        return BackendConnectionStatus(
          alive: true,
          ready: true,
          health: 'healthy',
          status: 'online',
          latencyMs: 0,
        );
      }

      final current = await fetchStatus();
      trace(
        'connect initial current alive=${current.alive} ready=${current.ready} healthy=${current.healthy} uiState=${current.uiState.name}',
      );
      if (current.ready && current.healthy) {
        trace('connect returns current because backend already healthy');
        return current;
      }

      final started = await _startRelayIfNeeded();
      trace('connect _startRelayIfNeeded result=$started');
      if (!started) {
        trace('connect failed: backend not reachable at $resolvedBaseUrl');
        throw StateError('backend not reachable at $resolvedBaseUrl');
      }

      final deadline = DateTime.now().add(const Duration(seconds: 20));
      while (DateTime.now().isBefore(deadline)) {
        final status = await fetchStatus();
        if (status.ready && status.healthy) {
          trace(
            'connect success after polling status alive=${status.alive} ready=${status.ready} healthy=${status.healthy} uiState=${status.uiState.name}',
          );
          return status;
        }
        await Future<void>.delayed(const Duration(milliseconds: 600));
      }

      final finalStatus = await fetchStatus();
      trace(
        'connect finalStatus alive=${finalStatus.alive} ready=${finalStatus.ready} healthy=${finalStatus.healthy} uiState=${finalStatus.uiState.name}',
      );
      if (finalStatus.ready && finalStatus.healthy) {
        return finalStatus;
      }
      trace('connect timeout while establishing a secure session');
      throw StateError('backend timeout while establishing a secure session');
    } finally {
      _connecting = false;
      trace('connect finally _connecting=false');
    }
  }

  Future<void> disconnect() async {
    trace('disconnect called process=${_process?.pid ?? 0}');
    final process = _process;
    if (process != null && process.pid != 0) {
      final didExit = process.kill();
      _process = null;
      trace('disconnect process kill result=$didExit');
      if (didExit) {
        return;
      }
    }
    _process = null;
    trace('disconnect complete');
  }

  Future<bool> _startRelayIfNeeded() async {
    trace('_startRelayIfNeeded reachable=${await _isReachable()}');
    if (await _isReachable()) {
      return true;
    }

    if (Platform.isAndroid || Platform.isIOS) {
      trace('_startRelayIfNeeded aborted: Android/iOS cannot start host relay');
      return false;
    }

    final cargoCommand = Platform.isWindows ? 'cargo.exe' : 'cargo';
    final process = await Process.start(
      cargoCommand,
      const ['run', '--bin', 'ghost-layer-relay'],
      workingDirectory: workingDirectory,
      environment: {
        ...Platform.environment,
        'GHOST_HEALTH_LISTEN_ADDRESS': '127.0.0.1:$healthPort',
      },
    );

    _process = process;
    final started = await Future<bool>.delayed(
      const Duration(milliseconds: 700),
      () => _isReachable(),
    );

    return started;
  }

  Future<bool> _isReachable() async {
    try {
      final response = await http
          .get(Uri.parse('$resolvedBaseUrl/live'))
          .timeout(const Duration(seconds: 2));
      return response.statusCode == 200;
    } catch (_) {
      return false;
    }
  }

  Map<String, dynamic>? _decodePayload(String rawBody) {
    final trimmed = rawBody.trim();
    if (trimmed.isEmpty) {
      return null;
    }
    try {
      final decoded = jsonDecode(trimmed);
      if (decoded is Map<String, dynamic>) {
        return decoded;
      }
      if (decoded is Map) {
        return decoded.map((key, value) => MapEntry(key.toString(), value));
      }
    } catch (_) {
      return null;
    }
    return null;
  }
}
