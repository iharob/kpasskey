package org.kpasskey.service

import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import androidx.lifecycle.LifecycleService
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch
import org.kpasskey.R
import org.kpasskey.container
import org.kpasskey.net.LinkState

/**
 * Keeps the link alive while the app is not on screen. The state itself belongs to
 * `LinkController`; this only holds the process up, mirrors the pending request into a
 * notification, and gives the notification's Deny action somewhere to land.
 */
class LinkService : LifecycleService() {

    private val notifications by lazy { Notifications(this) }

    override fun onCreate() {
        super.onCreate()
        notifications.ensureChannels()
        startForeground(
            Notifications.ID_ONGOING,
            notifications.ongoing(getString(R.string.status_connecting)),
            ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
        )

        val controller = container.link
        controller.start()

        lifecycleScope.launch {
            controller.pending.collect { request ->
                if (request == null) {
                    notifications.clearApproval()
                } else {
                    notifications.raiseApproval(request)
                }
            }
        }

        lifecycleScope.launch {
            combine(controller.state, container.desktops.paired) { state, desktop ->
                when (state) {
                    LinkState.Connected -> getString(R.string.status_connected, desktop?.hostName.orEmpty())
                    LinkState.Connecting -> getString(R.string.status_connecting)
                    LinkState.Waiting -> getString(R.string.status_waiting)
                }
            }.collect { status ->
                notifications.ongoing(status).let { notification ->
                    startForeground(
                        Notifications.ID_ONGOING,
                        notification,
                        ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
                    )
                }
            }
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        if (intent?.action == ACTION_DENY) {
            val controller = container.link
            controller.pending.value?.let { request ->
                controller.deny(request, "user-cancelled")
            }
        }
        return START_STICKY
    }

    override fun onDestroy() {
        container.link.stop()
        super.onDestroy()
    }

    companion object {
        private const val ACTION_DENY = "org.kpasskey.DENY"

        fun start(context: Context) {
            context.startForegroundService(Intent(context, LinkService::class.java))
        }

        fun denyIntent(context: Context): Intent =
            Intent(context, LinkService::class.java).setAction(ACTION_DENY)
    }
}
