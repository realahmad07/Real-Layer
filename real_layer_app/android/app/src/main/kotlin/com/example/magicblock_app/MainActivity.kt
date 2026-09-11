package com.example.magicblock_app

import android.content.Intent
import android.net.VpnService
import android.view.MotionEvent
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import android.util.Log


class MainActivity : FlutterActivity() {

    private val CHANNEL = "real_layer/vpn"
    private val VPN_REQUEST_CODE = 1001

    override fun onCreate(savedInstanceState: android.os.Bundle?) {
        Log.i("MainActivity", "ANDROID_MAIN_ACTIVITY_ON_CREATE_ENTERED")
        super.onCreate(savedInstanceState)
        Log.i("MainActivity", "ANDROID_MAIN_ACTIVITY_ON_CREATE_COMPLETED")
    }

    override fun dispatchTouchEvent(event: MotionEvent?): Boolean {
        Log.i(
            "MainActivity",
            "dispatchTouchEvent action=${event?.actionMasked} x=${event?.x} y=${event?.y} rawX=${event?.rawX} rawY=${event?.rawY}",
        )
        return super.dispatchTouchEvent(event)
    }

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        Log.i("MainActivity", "ANDROID_FLUTTER_ENGINE_CONFIGURE_ENTERED")
        super.configureFlutterEngine(flutterEngine)

        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, CHANNEL)
            .setMethodCallHandler { call, result ->
                Log.i("MainActivity", "VPN_METHOD_START_RECEIVED method=${call.method} channel=$CHANNEL")
                when (call.method) {
                    "startVpn" -> {
                        Log.i("MainActivity", "VPN_CONNECT_ACTION")
                        Log.i("MainActivity", "VPN_PREPARE_CALL")
                        val permIntent = VpnService.prepare(this)
                        if (permIntent != null) {
                            Log.i("MainActivity", "VPN_PREPARE_RESULT intent")
                            Log.i("MainActivity", "VPN_PERMISSION_REQUIRED")
                            startActivityForResult(permIntent, VPN_REQUEST_CODE)
                            Log.i("MainActivity", "VPN_PERMISSION_ACTIVITY_LAUNCHED")
                        } else {
                            Log.i("MainActivity", "VPN_PREPARE_RESULT null")
                            Log.i("MainActivity", "VPN_SERVICE_START_REQUEST")
                            doStartVpn()
                        }
                        result.success(null)
                    }
                    "stopVpn" -> {
                        val intent = Intent(this, RealLayerVpnService::class.java)
                        intent.action = "STOP"
                        startService(intent)
                        result.success(null)
                    }
                    else -> result.notImplemented()
                }
            }
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == VPN_REQUEST_CODE && resultCode == RESULT_OK) {
            Log.i("MainActivity", "VPN_PERMISSION_RESULT code=$resultCode")
            Log.i("MainActivity", "VPN_PREPARE_RESULT granted")
            Log.i("MainActivity", "VPN_SERVICE_START_REQUEST")
            Log.i("MainActivity", "VPN_PERMISSION_GRANTED")
            doStartVpn()
        }
    }

    private fun doStartVpn() {
        Log.i("MainActivity", "VPN_SERVICE_START_REQUEST")
        val intent = Intent(this, RealLayerVpnService::class.java)
        intent.action = "START"
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.O) {
            startForegroundService(intent)
        } else {
            startService(intent)
        }
    }
}
