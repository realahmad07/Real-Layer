import 'dart:io';
import 'backend_bridge_cli.dart';

void main() async {
  final bridge = RealLayerBackendBridge(
    baseUrl: 'http://127.0.0.1:8081',
    repoRoot: 'C:/Users/Ali Computers/Real - Layer/Real-Layer',
  );

  print('--- Starting connection trace ---');
  try {
    print('Calling connect()');
    final status = await bridge.connect();
    print('connect() finished. Status: ${status.uiState.name}');
    
    print('Waiting 32 seconds to ensure stability (passing the 10s and 30s marks)...');
    for (var i = 1; i <= 32; i++) {
      await Future.delayed(Duration(seconds: 1));
      final s = await bridge.fetchStatus();
      print('  [$i s] health=${s.health} status=${s.status} ready=${s.ready} alive=${s.alive} -> uiState=${s.uiState.name}');
    }

    print('Calling disconnect()');
    await bridge.disconnect();
    
    await Future.delayed(Duration(seconds: 2));
    final s2 = await bridge.fetchStatus();
    print('After disconnect status: ${s2.uiState.name}');

    print('Calling reconnect()');
    final status2 = await bridge.connect();
    print('reconnect() finished. Status: ${status2.uiState.name}');

    print('Trace complete.');
    exit(0);
  } catch (e) {
    print('Error: $e');
    exit(1);
  }
}
