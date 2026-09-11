import 'package:flutter_test/flutter_test.dart';
import 'package:magicblock_app/backend_bridge.dart';

void main() {
  test('parses live backend status payload into connection state', () {
    final status = BackendConnectionStatus.fromJson({
      'alive': true,
      'ready': true,
      'health': 'healthy',
      'latency_ms': 42,
      'status': 'connected',
    });

    expect(status.alive, isTrue);
    expect(status.ready, isTrue);
    expect(status.health, 'healthy');
    expect(status.latencyMs, 42);
    expect(status.status, 'connected');
  });

  test(
    'disconnected backend status does not map to disconnecting UI state',
    () {
      final status = BackendConnectionStatus.fromJson({
        'alive': false,
        'ready': false,
        'health': 'unavailable',
        'status': 'disconnected',
      });

      expect(status.uiState, BackendRuntimeConnectionState.disconnected);
      expect(status.statusLabel, 'Disconnected');
    },
  );

  test('disconnecting backend status maps only to disconnecting UI state', () {
    final status = BackendConnectionStatus.fromJson({
      'alive': true,
      'ready': false,
      'health': 'unavailable',
      'status': 'disconnecting',
    });

    expect(status.uiState, BackendRuntimeConnectionState.disconnecting);
    expect(status.statusLabel, 'Disconnecting');
  });

  test(
    'ready and health payloads without alive still map to connected state',
    () {
      final readyStatus = BackendConnectionStatus.fromJson({
        'ready': true,
        'health': 'healthy',
      });
      final healthStatus = BackendConnectionStatus.fromJson({
        'health': 'healthy',
      });

      expect(readyStatus.uiState, BackendRuntimeConnectionState.connected);
      expect(healthStatus.uiState, BackendRuntimeConnectionState.connected);
      expect(readyStatus.statusLabel, 'Protected');
      expect(healthStatus.statusLabel, 'Protected');
    },
  );
}
