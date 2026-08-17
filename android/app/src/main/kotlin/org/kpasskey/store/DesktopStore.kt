package org.kpasskey.store

import android.content.Context
import android.os.Build
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

data class PairedDesktop(
    val deviceId: String,
    val hostName: String,
    val address: String,
    val port: Int,
    val pairedAt: Long,
)

/**
 * The one desktop this phone is paired with, and the name this phone reports to it.
 * Nothing secret lives here — the private key never leaves the keystore — so plain
 * preferences are the right weight.
 */
class DesktopStore(context: Context) {

    private val preferences = context.getSharedPreferences(FILE, Context.MODE_PRIVATE)
    private val mutablePaired = MutableStateFlow(read())
    val paired: StateFlow<PairedDesktop?> = mutablePaired.asStateFlow()

    private val mutableName = MutableStateFlow(preferences.getString(KEY_NAME, null) ?: Build.MODEL)
    val phoneName: StateFlow<String> = mutableName.asStateFlow()

    fun save(desktop: PairedDesktop) {
        preferences.edit()
            .putString(KEY_ID, desktop.deviceId)
            .putString(KEY_HOST, desktop.hostName)
            .putString(KEY_ADDRESS, desktop.address)
            .putInt(KEY_PORT, desktop.port)
            .putLong(KEY_PAIRED_AT, desktop.pairedAt)
            .apply()
        mutablePaired.value = desktop
    }

    fun forget() {
        preferences.edit()
            .remove(KEY_ID)
            .remove(KEY_HOST)
            .remove(KEY_ADDRESS)
            .remove(KEY_PORT)
            .remove(KEY_PAIRED_AT)
            .apply()
        mutablePaired.value = null
    }

    fun rename(name: String) {
        val trimmed = name.trim().ifEmpty { Build.MODEL }
        preferences.edit().putString(KEY_NAME, trimmed).apply()
        mutableName.value = trimmed
    }

    private fun read(): PairedDesktop? {
        val id = preferences.getString(KEY_ID, null) ?: return null
        return PairedDesktop(
            deviceId = id,
            hostName = preferences.getString(KEY_HOST, null).orEmpty(),
            address = preferences.getString(KEY_ADDRESS, null).orEmpty(),
            port = preferences.getInt(KEY_PORT, 0),
            pairedAt = preferences.getLong(KEY_PAIRED_AT, 0),
        )
    }

    private companion object {
        const val FILE = "kpasskey"
        const val KEY_ID = "desktop.id"
        const val KEY_HOST = "desktop.host"
        const val KEY_ADDRESS = "desktop.address"
        const val KEY_PORT = "desktop.port"
        const val KEY_PAIRED_AT = "desktop.pairedAt"
        const val KEY_NAME = "phone.name"
    }
}
