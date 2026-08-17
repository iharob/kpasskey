package org.kpasskey

import android.app.Application
import android.content.Context
import org.kpasskey.crypto.AuthKeyStore
import org.kpasskey.link.LinkController
import org.kpasskey.store.DesktopStore

/**
 * Everything with a lifetime longer than a screen, constructed in one place and owned by the
 * process. Reached through [container]; nothing here is a global, and nothing constructs
 * itself on first use.
 */
class Container(context: Context) {
    val keys: AuthKeyStore = AuthKeyStore.open()
    val desktops: DesktopStore = DesktopStore(context)
    val link: LinkController = LinkController(keys, desktops)
}

class KpkApplication : Application() {

    lateinit var container: Container
        private set

    override fun onCreate() {
        super.onCreate()
        container = Container(this)
    }
}

/** The single way a screen or service reaches shared state. */
val Context.container: Container
    get() = (applicationContext as KpkApplication).container
