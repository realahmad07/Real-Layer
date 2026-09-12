import 'dart:async';
import 'dart:io';
import 'dart:ui';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'backend_bridge.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  FlutterError.onError = (details) {
    FlutterError.presentError(details);
    debugPrint(
      '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} FLUTTER_ERROR ${details.exceptionAsString()}',
    );
  };
  PlatformDispatcher.instance.onError = (error, stack) {
    debugPrint(
      '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} FLUTTER_PLATFORM_ERROR $error',
    );
    return true;
  };

  debugPrint(
    '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} FLUTTER_MAIN_ENTERED',
  );
  runApp(const MyApp());
}

class MyApp extends StatelessWidget {
  const MyApp({super.key});

  @override
  Widget build(BuildContext context) {
    debugPrint(
      '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} MYAPP_BUILD_ENTERED',
    );
    return MaterialApp(
      title: 'RealLayer VPN',
      theme: ThemeData(
        brightness: Brightness.dark,
        scaffoldBackgroundColor: Colors.black,
      ),
      home: const LoginScreen(),
      debugShowCheckedModeBanner: false,
    );
  }
}

class MagicBlockScreen extends StatefulWidget {
  const MagicBlockScreen({super.key});

  @override
  State<MagicBlockScreen> createState() => _MagicBlockScreenState();
}

class _MagicBlockScreenState extends State<MagicBlockScreen>
    with TickerProviderStateMixin {
  final RealLayerBackendBridge _bridge = RealLayerBackendBridge();
  final MethodChannel _vpnChannel = const MethodChannel('real_layer/vpn');
  Timer? _statusTimer;

  late AnimationController _connectController;
  late AnimationController _pulseController;
  late Animation<double> _scaleAnim;
  late Animation<double> _pulseAnim;

  BackendConnectionStatus _status = BackendConnectionStatus.empty();
  BackendRuntimeConnectionState _connectionState =
      BackendRuntimeConnectionState.disconnected;
  bool _userDisconnected = true;

  bool get _isConnected =>
      _connectionState == BackendRuntimeConnectionState.connected;
  bool get _isConnecting =>
      _connectionState == BackendRuntimeConnectionState.connecting;
  bool get _isDisconnecting =>
      _connectionState == BackendRuntimeConnectionState.disconnecting;

  @override
  void initState() {
    super.initState();
    debugPrint(
      '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} APP_MAIN_SCREEN_REACHED',
    );

    _connectController = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 180),
    );
    _pulseController = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 900),
    );

    _scaleAnim = Tween<double>(begin: 1.0, end: 0.93).animate(
      CurvedAnimation(parent: _connectController, curve: Curves.easeInOut),
    );
    _pulseAnim = Tween<double>(begin: 1.0, end: 1.08).animate(
      CurvedAnimation(parent: _pulseController, curve: Curves.easeInOut),
    );

    _refreshBackendStatus();
    _statusTimer = Timer.periodic(const Duration(seconds: 2), (_) {
      if (mounted) {
        unawaited(_refreshBackendStatus());
      }
    });
  }

  @override
  void dispose() {
    _statusTimer?.cancel();
    _connectController.dispose();
    _pulseController.dispose();
    super.dispose();
  }

  Future<void> _refreshBackendStatus() async {
    if (!Platform.isWindows && !Platform.isMacOS && !Platform.isLinux) {
      if (_connectionState == BackendRuntimeConnectionState.connected ||
          _connectionState == BackendRuntimeConnectionState.connecting) {
        return;
      }
    }
    debugPrint(
      '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} _refreshBackendStatus start',
    );
    final updated = await _bridge.fetchStatus().catchError(
      (_) => BackendConnectionStatus.empty(),
    );
    if (!mounted) return;

    debugPrint(
      '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} _refreshBackendStatus updated alive=${updated.alive} ready=${updated.ready} health=${updated.health} status=${updated.status} uiState=${updated.uiState.name}',
    );

    // If the user explicitly disconnected, don't auto-promote back to connected
    // from background health polling alone.
    if (_userDisconnected &&
        updated.uiState == BackendRuntimeConnectionState.connected) {
      debugPrint(
        '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} _refreshBackendStatus suppressed: _userDisconnected=true, not auto-connecting',
      );
      setState(() {
        _status = updated;
        // Keep disconnected state
        _connectionState = BackendRuntimeConnectionState.disconnected;
      });
      debugPrint(
        '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} _connectionState set to disconnected (user guard)',
      );
      return;
    }

    if (!Platform.isWindows && !Platform.isMacOS && !Platform.isLinux) {
      if (_connectionState == BackendRuntimeConnectionState.connected ||
          _connectionState == BackendRuntimeConnectionState.connecting) {
        debugPrint(
          '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} _refreshBackendStatus suppressed: mobile state is already ${_connectionState.name}',
        );
        return;
      }
    }

    setState(() {
      _status = updated;
      _connectionState = updated.uiState;
    });
    debugPrint(
      '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} _connectionState set to ${_connectionState.name}',
    );
  }

  Future<void> _onConnectTap() async {
    debugPrint(
      '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} ON_CONNECT_TAP_ENTERED currentState=${_connectionState.name}',
    );
    debugPrint(
      '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} CONNECT_BUTTON_TAPPED currentState=${_connectionState.name}',
    );
    await _connectController.forward();
    await _connectController.reverse();

    if (_isConnecting || _isDisconnecting) {
      debugPrint(
        '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} CONNECT tap ignored while ${_connectionState.name}',
      );
      return;
    }

    if (_isConnected) {
      debugPrint(
        '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} user started DISCONNECT path',
      );
      setState(() {
        _userDisconnected = true;
        _connectionState = BackendRuntimeConnectionState.disconnecting;
      });
      debugPrint(
        '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} _connectionState set to ${_connectionState.name}',
      );

      try {
        // Stop the Android VPN tunnel
        try {
          await _vpnChannel.invokeMethod<void>('stopVpn');
        } catch (vpnErr) {
          debugPrint(
            '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} stopVpn channel error=$vpnErr',
          );
        }
        await _bridge.disconnect();
        await _refreshBackendStatus();
      } catch (_) {
        if (mounted) {
          setState(() {
            _connectionState = BackendRuntimeConnectionState.error;
          });
          debugPrint(
            '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} _connectionState set to ${_connectionState.name} from disconnect exception',
          );
        }
      }
      return;
    }

    if (!mounted) return;

    setState(() {
      _userDisconnected = false;
      _connectionState = BackendRuntimeConnectionState.connecting;
    });
    debugPrint(
      '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} _connectionState set to ${_connectionState.name}',
    );
    _pulseController.repeat(reverse: true);

    try {
      // Start the Android VPN tunnel (no-op on non-Android)
      try {
        debugPrint(
          '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} VPN_METHOD_INVOKE_START method=startVpn channel=real_layer/vpn',
        );
        await _vpnChannel.invokeMethod<void>('startVpn');
        debugPrint(
          '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} VPN_METHOD_INVOKE_RESULT method=startVpn success',
        );
      } catch (vpnErr) {
        debugPrint(
          '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} VPN_METHOD_INVOKE_RESULT method=startVpn error=$vpnErr',
        );
      }
      final connectedStatus = await _bridge.connect();
      if (!mounted) return;
      debugPrint(
        '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} connect returned alive=${connectedStatus.alive} ready=${connectedStatus.ready} health=${connectedStatus.health} status=${connectedStatus.status} uiState=${connectedStatus.uiState.name}',
      );
      setState(() {
        _status = connectedStatus;
        _connectionState = connectedStatus.uiState;
      });
      debugPrint(
        '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} _connectionState set to ${_connectionState.name} after connect',
      );
    } catch (error) {
      if (!mounted) return;
      debugPrint(
        '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} connect exception $error',
      );
      setState(() {
        _status = BackendConnectionStatus(
          alive: false,
          ready: false,
          health: 'unavailable',
          latencyMs: 0,
          status: 'error',
          message: error.toString(),
        );
        _connectionState = BackendRuntimeConnectionState.error;
      });
      debugPrint(
        '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} _connectionState set to ${_connectionState.name} from error',
      );
    } finally {
      _pulseController.stop();
      _pulseController.reset();
      debugPrint(
        '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} connect finally pulse stopped',
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    debugPrint(
      '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} CONNECT_BUTTON_VISIBLE',
    );
    final h = MediaQuery.of(context).size.height;
    final w = MediaQuery.of(context).size.width;
    final connectRect = Rect.fromLTWH(w * 0.5 - 110, h * 0.54, 220, 80);

    return Listener(
      onPointerUp: (event) {
        final local = event.localPosition;
        if (connectRect.contains(local)) {
          debugPrint(
            '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} CONNECT_POINTER_UP',
          );
          _onConnectTap();
        }
      },
      child: Scaffold(
        backgroundColor: Colors.black,
        body: SafeArea(
          child: Stack(
            children: [
              Positioned(
                top: h * 0.18,
                left: w * 0.05,
                right: w * 0.05,
                child: Container(
                  height: 300,
                  decoration: BoxDecoration(
                    shape: BoxShape.circle,
                    boxShadow: [
                      BoxShadow(
                        color:
                            (_isConnected
                                    ? const Color(0xFF28B78D)
                                    : const Color(0xFF8B47FF))
                                .withOpacity(0.14),
                        blurRadius: 180,
                        spreadRadius: 80,
                      ),
                    ],
                  ),
                ),
              ),
              Column(
                crossAxisAlignment: CrossAxisAlignment.center,
                children: [
                  SizedBox(height: h * 0.04),
                  ShaderMask(
                    shaderCallback: (bounds) => const LinearGradient(
                      colors: [Color(0xFFB158FF), Color(0xFF8270FF)],
                      begin: Alignment.centerLeft,
                      end: Alignment.centerRight,
                    ).createShader(bounds),
                    child: const Text(
                      'REALLAYER',
                      style: TextStyle(
                        fontSize: 26,
                        fontWeight: FontWeight.w900,
                        letterSpacing: 6,
                        color: Colors.white,
                      ),
                    ),
                  ),
                  const SizedBox(height: 12),
                  const AnimatedSolanaLogo(),
                  const SizedBox(height: 20),
                  AnimatedBuilder(
                    animation: _pulseAnim,
                    builder: (context, child) {
                      return Transform.scale(
                        scale: _isConnecting ? _pulseAnim.value : 1.0,
                        child: child,
                      );
                    },
                    child: Image.asset(
                      'assets/box.png',
                      height: h * 0.22,
                      fit: BoxFit.contain,
                    ),
                  ),
                  const SizedBox(height: 12),
                  Text(
                    _status.statusLabel,
                    style: const TextStyle(
                      color: Colors.white,
                      fontSize: 12,
                      fontWeight: FontWeight.w600,
                      letterSpacing: 3,
                    ),
                  ),
                  const SizedBox(height: 12),
                  GestureDetector(
                    behavior: HitTestBehavior.opaque,
                    onTap: () {
                      debugPrint(
                        '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} CONNECT_BUTTON_TAPPED',
                      );
                      _onConnectTap();
                    },
                    child: SizedBox(
                      width: 220,
                      height: 80,
                      child: DecoratedBox(
                        decoration: BoxDecoration(
                          color: const Color(0xFFFFD400),
                          borderRadius: BorderRadius.circular(12),
                          boxShadow: const [
                            BoxShadow(
                              color: Color(0xFFFFD400),
                              blurRadius: 20,
                              spreadRadius: 2,
                              offset: Offset(0, 4),
                            ),
                          ],
                        ),
                        child: const Center(
                          child: Text(
                            'CONNECT',
                            style: TextStyle(
                              fontSize: 22,
                              fontWeight: FontWeight.bold,
                              letterSpacing: 2,
                              color: Colors.black,
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                  const SizedBox(height: 20),
                  Expanded(
                    child: Padding(
                      padding: const EdgeInsets.only(left: 32),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        mainAxisAlignment: MainAxisAlignment.spaceEvenly,
                        children: [
                          _buildIconRow(
                            Icons.language,
                            'Server ${_status.alive ? 'online' : 'offline'}',
                          ),
                          _buildIconRow(
                            Icons.flash_on,
                            _status.latencyMs > 0
                                ? '${_status.latencyMs}ms'
                                : 'Latency unavailable',
                          ),
                          _buildIconRow(Icons.trending_up, _status.health),
                          _buildIconRow(
                            Icons.verified_user_outlined,
                            _status.ready ? 'Protected' : 'Unprotected',
                          ),
                        ],
                      ),
                    ),
                  ),
                  const Text(
                    'POWERED  MAGICBLOCK',
                    style: TextStyle(
                      fontSize: 10,
                      fontWeight: FontWeight.w500,
                      letterSpacing: 3,
                      color: Color(0xFF666666),
                    ),
                  ),
                  const SizedBox(height: 16),
                ],
              ),
              Positioned(
                top: 10,
                right: 10,
                child: IconButton(
                  icon: const Icon(Icons.logout, color: Color(0xFF666666)),
                  onPressed: () {
                    Navigator.of(context).pushReplacement(
                      MaterialPageRoute(
                        builder: (context) => const LoginScreen(),
                      ),
                    );
                  },
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget _buildIconRow(IconData icon, String label) {
    return Row(
      children: [
        AnimatedContainer(
          duration: const Duration(milliseconds: 400),
          child: Icon(
            icon,
            color: _isConnected
                ? const Color(0xFF28E89B)
                : const Color(0xFFC6B5FF),
            size: 24,
          ),
        ),
        const SizedBox(width: 20),
        Text(
          label,
          style: const TextStyle(
            fontSize: 14,
            fontWeight: FontWeight.w600,
            letterSpacing: 2,
            color: Colors.white70,
          ),
        ),
      ],
    );
  }
}

class AnimatedSolanaLogo extends StatefulWidget {
  const AnimatedSolanaLogo({super.key});

  @override
  State<AnimatedSolanaLogo> createState() => _AnimatedSolanaLogoState();
}

class _AnimatedSolanaLogoState extends State<AnimatedSolanaLogo>
    with SingleTickerProviderStateMixin {
  late final AnimationController _controller;
  late final Animation<double> _shimmer;

  @override
  void initState() {
    super.initState();
    _controller = AnimationController(
      vsync: this,
      duration: const Duration(seconds: 2),
    )..repeat();
    _shimmer = Tween<double>(
      begin: -1.5,
      end: 2.5,
    ).animate(CurvedAnimation(parent: _controller, curve: Curves.easeInOut));
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: _shimmer,
      builder: (context, child) {
        return Column(
          children: [
            _buildBar(
              [const Color(0xFF19FB9B), const Color(0xFF00C2FF)],
              true,
              _shimmer.value,
            ),
            const SizedBox(height: 8),
            _buildBar(
              [const Color(0xFF00C2FF), const Color(0xFF9945FF)],
              false,
              _shimmer.value,
            ),
            const SizedBox(height: 8),
            _buildBar(
              [const Color(0xFF9945FF), const Color(0xFF19FB9B)],
              true,
              _shimmer.value,
            ),
          ],
        );
      },
    );
  }

  Widget _buildBar(List<Color> colors, bool skewLeft, double shimmerPos) {
    return Transform(
      transform: Matrix4.skewX(skewLeft ? -0.45 : 0.45),
      child: ClipRRect(
        borderRadius: BorderRadius.circular(2),
        child: SizedBox(
          width: 160,
          height: 26,
          child: Stack(
            children: [
              Container(
                decoration: BoxDecoration(
                  gradient: LinearGradient(
                    colors: colors,
                    begin: Alignment.centerLeft,
                    end: Alignment.centerRight,
                  ),
                ),
              ),
              Positioned.fill(
                child: Transform.translate(
                  offset: Offset(shimmerPos * 160, 0),
                  child: Container(
                    width: 70,
                    decoration: BoxDecoration(
                      gradient: LinearGradient(
                        colors: [
                          Colors.white.withOpacity(0.0),
                          Colors.white.withOpacity(0.4),
                          Colors.white.withOpacity(0.0),
                        ],
                        stops: const [0.0, 0.5, 1.0],
                      ),
                    ),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

// ─── Login Screen ──────────────────────────────────────────────────────────────

class LoginScreen extends StatefulWidget {
  const LoginScreen({super.key});

  @override
  State<LoginScreen> createState() => _LoginScreenState();
}

class _LoginScreenState extends State<LoginScreen> {
  final _emailController = TextEditingController();
  final _passwordController = TextEditingController();

  void _login() {
    // In a real app, validate and authenticate here
    Navigator.of(context).pushReplacement(
      MaterialPageRoute(builder: (context) => const MagicBlockScreen()),
    );
  }

  @override
  Widget build(BuildContext context) {
    debugPrint(
      '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} LOGIN_SCREEN_BUILD_ENTERED',
    );
    WidgetsBinding.instance.addPostFrameCallback((_) {
      debugPrint(
        '[REAL_LAYER_TRACE] ${DateTime.now().toIso8601String()} FLUTTER_FIRST_FRAME_RENDERED',
      );
    });

    final h = MediaQuery.of(context).size.height;
    final w = MediaQuery.of(context).size.width;

    return Scaffold(
      backgroundColor: Colors.black,
      body: SafeArea(
        child: Stack(
          children: [
            // Ambient glow matching the theme
            Positioned(
              top: h * 0.1,
              left: w * 0.05,
              right: w * 0.05,
              child: Container(
                height: 300,
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  boxShadow: [
                    BoxShadow(
                      color: const Color(0xFF8B47FF).withOpacity(0.12),
                      blurRadius: 200,
                      spreadRadius: 100,
                    ),
                  ],
                ),
              ),
            ),

            SingleChildScrollView(
              padding: const EdgeInsets.symmetric(horizontal: 40),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.center,
                children: [
                  SizedBox(height: h * 0.12),

                  // ── REALLAYER ──────────────────────────────────────
                  ShaderMask(
                    shaderCallback: (bounds) => const LinearGradient(
                      colors: [Color(0xFFB158FF), Color(0xFF8270FF)],
                      begin: Alignment.centerLeft,
                      end: Alignment.centerRight,
                    ).createShader(bounds),
                    child: const Text(
                      'REALLAYER',
                      style: TextStyle(
                        fontSize: 28,
                        fontWeight: FontWeight.w900,
                        letterSpacing: 6,
                        color: Colors.white,
                      ),
                    ),
                  ),

                  const SizedBox(height: 10),
                  const Text(
                    'SECURE VPN',
                    style: TextStyle(
                      fontSize: 12,
                      fontWeight: FontWeight.w600,
                      letterSpacing: 4,
                      color: Color(0xFF888888),
                    ),
                  ),

                  SizedBox(height: h * 0.1),

                  // Email Field
                  _buildTextField(
                    controller: _emailController,
                    hint: 'EMAIL',
                    icon: Icons.email_outlined,
                  ),

                  const SizedBox(height: 20),

                  // Password Field
                  _buildTextField(
                    controller: _passwordController,
                    hint: 'PASSWORD',
                    icon: Icons.lock_outline,
                    obscureText: true,
                  ),

                  const SizedBox(height: 40),

                  // Login Button
                  GestureDetector(
                    onTap: _login,
                    child: Container(
                      width: double.infinity,
                      height: 50,
                      decoration: BoxDecoration(
                        borderRadius: BorderRadius.circular(4),
                        gradient: const LinearGradient(
                          colors: [Color(0xFF8F5AFF), Color(0xFF28B78D)],
                          begin: Alignment.centerLeft,
                          end: Alignment.centerRight,
                        ),
                        boxShadow: [
                          BoxShadow(
                            color: const Color(0xFF8F5AFF).withOpacity(0.3),
                            blurRadius: 15,
                            offset: const Offset(0, 4),
                          ),
                        ],
                      ),
                      child: const Center(
                        child: Text(
                          'LOGIN',
                          style: TextStyle(
                            fontSize: 14,
                            fontWeight: FontWeight.bold,
                            letterSpacing: 3,
                            color: Colors.white,
                          ),
                        ),
                      ),
                    ),
                  ),

                  const SizedBox(height: 30),

                  // Forgot Password / Sign up
                  TextButton(
                    onPressed: () {},
                    child: const Text(
                      'FORGOT PASSWORD?',
                      style: TextStyle(
                        fontSize: 11,
                        letterSpacing: 1.5,
                        color: Color(0xFFAA99DD),
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildTextField({
    required TextEditingController controller,
    required String hint,
    required IconData icon,
    bool obscureText = false,
  }) {
    return Container(
      height: 55,
      decoration: BoxDecoration(
        color: const Color(0xFF0A0F14), // Very dark tint
        borderRadius: BorderRadius.circular(6),
        border: Border.all(color: const Color(0xFF8B47FF).withOpacity(0.3)),
      ),
      child: Row(
        children: [
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 16),
            child: Icon(icon, color: const Color(0xFF8B47FF), size: 20),
          ),
          Expanded(
            child: TextField(
              controller: controller,
              obscureText: obscureText,
              style: const TextStyle(color: Colors.white, fontSize: 14),
              decoration: InputDecoration(
                border: InputBorder.none,
                hintText: hint,
                hintStyle: const TextStyle(
                  color: Color(0xFF666666),
                  letterSpacing: 2,
                  fontSize: 12,
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}
