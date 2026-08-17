package org.kpasskey.service

import android.Manifest
import android.app.Notification
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationChannelCompat
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import org.kpasskey.R
import org.kpasskey.net.Approval
import org.kpasskey.ui.ApprovalActivity

class Notifications(private val context: Context) {

    private val manager = NotificationManagerCompat.from(context)

    fun ensureChannels() {
        manager.createNotificationChannelsCompat(
            listOf(
                NotificationChannelCompat.Builder(CHANNEL_LINK, NotificationManagerCompat.IMPORTANCE_LOW)
                    .setName(context.getString(R.string.channel_link))
                    .setDescription(context.getString(R.string.channel_link_description))
                    .build(),
                NotificationChannelCompat.Builder(CHANNEL_APPROVAL, NotificationManagerCompat.IMPORTANCE_HIGH)
                    .setName(context.getString(R.string.channel_approval))
                    .setDescription(context.getString(R.string.channel_approval_description))
                    .build(),
            ),
        )
    }

    /** The notification that keeps the service in the foreground. Deliberately quiet. */
    fun ongoing(status: String): Notification =
        NotificationCompat.Builder(context, CHANNEL_LINK)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(context.getString(R.string.app_name))
            .setContentText(status)
            .setOngoing(true)
            .setShowWhen(false)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setContentIntent(activityIntent(ApprovalActivity.intent(context), REQUEST_OPEN))
            .build()

    fun raiseApproval(request: Approval) {
        if (!allowed()) {
            return
        }

        val open = activityIntent(ApprovalActivity.intent(context), REQUEST_APPROVE)
        val builder =
            NotificationCompat.Builder(context, CHANNEL_APPROVAL)
                .setSmallIcon(R.drawable.ic_notification)
                .setContentTitle(request.sentence())
                .setContentText(context.getString(R.string.notification_approval_body))
                .setCategory(NotificationCompat.CATEGORY_CALL)
                .setPriority(NotificationCompat.PRIORITY_HIGH)
                .setAutoCancel(true)
                .setOnlyAlertOnce(true)
                .setContentIntent(open)
                .addAction(
                    R.drawable.ic_notification,
                    context.getString(R.string.action_deny),
                    PendingIntent.getService(
                        context,
                        REQUEST_DENY,
                        LinkService.denyIntent(context),
                        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
                    ),
                )

        // Android 14+ default-denies USE_FULL_SCREEN_INTENT to apps that are not calling or
        // alarm apps. Without the check the notification is posted with the intent silently
        // ignored; with it, the HIGH-importance channel still produces a heads-up.
        if (manager.canUseFullScreenIntent()) {
            builder.setFullScreenIntent(open, true)
        }

        manager.notify(ID_APPROVAL, builder.build())
    }

    fun clearApproval() {
        manager.cancel(ID_APPROVAL)
    }

    fun canUseFullScreenIntent(): Boolean = manager.canUseFullScreenIntent()

    /** The permission only exists from API 33; below that notifications are on by default. */
    private fun allowed(): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
            ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED

    private fun activityIntent(intent: Intent, request: Int): PendingIntent =
        PendingIntent.getActivity(
            context,
            request,
            intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )

    companion object {
        const val CHANNEL_LINK = "link"
        const val CHANNEL_APPROVAL = "approval"
        const val ID_ONGOING = 1
        const val ID_APPROVAL = 2

        private const val REQUEST_OPEN = 10
        private const val REQUEST_APPROVE = 11
        private const val REQUEST_DENY = 12
    }
}
