package com.example.magicblock_app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Intent
import android.net.VpnService
import android.os.Build
import android.os.ParcelFileDescriptor
import android.util.Log

class RealLayerVpnService : VpnService() {

    companion object {
        private const val TAG = "RealLayerVpnService"
        private const val CHANNEL_ID = "real_layer_vpn"
        private const val NOTIF_ID = 1001

        // Native JNI
        init {
            System.loadLibrary("ghost_layer_client")
        }

        @JvmStatic
        external fun startCore(fd: Int)

        @JvmStatic
        external fun stopCore()
    }

    private var tunInterface: ParcelFileDescriptor? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        Log.i(TAG, "onStartCommand action=${intent?.action}")
        when (intent?.action) {
            "START" -> startVpn()
            "STOP"  -> stopVpn()
        }
        return START_STICKY
    }

    private fun startVpn() {
        Log.i(TAG, "VPN_SERVICE_START")
        Log.i(TAG, "startVpn: building TUN interface")
        createNotificationChannel()
        startForeground(NOTIF_ID, buildNotification("Real Layer VPN Active"))

        val builder = Builder()
            .setSession("Real Layer")
            .addAddress("10.8.0.1", 24)
            .addRoute("0.0.0.0", 0)
            .addDnsServer("8.8.8.8")
            .setMtu(1500)

        tunInterface = builder.establish()
        val pfd = tunInterface ?: run {
            Log.e(TAG, "startVpn: failed to establish TUN")
            stopSelf()
            return
        }

        val fd = pfd.fd
        Log.i(TAG, "VPN_TUN_ESTABLISHED fd=$fd")
        Log.i(TAG, "VPN_RUST_START fd=$fd")
        println("VPN_RUST_START fd=$fd")
        startCore(fd)
    }

    private fun stopVpn() {
        Log.i(TAG, "stopVpn: stopping core and closing TUN")
        stopCore()
        tunInterface?.close()
        tunInterface = null
        stopForeground(true)
        stopSelf()
    }

    override fun onRevoke() {
        Log.i(TAG, "onRevoke: VPN revoked by system")
        stopVpn()
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                "Real Layer VPN",
                NotificationManager.IMPORTANCE_LOW
            )
            val nm = getSystemService(NotificationManager::class.java)
            nm.createNotificationChannel(channel)
        }
    }

    private fun buildNotification(text: String): Notification {
        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(this, CHANNEL_ID)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(this)
        }
        return builder
            .setContentTitle("Real Layer VPN")
            .setContentText(text)
            .setSmallIcon(android.R.drawable.ic_lock_lock)
            .build()
    }
}
