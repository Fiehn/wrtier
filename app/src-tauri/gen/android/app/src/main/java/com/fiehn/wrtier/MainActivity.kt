package com.fiehn.wrtier

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.os.Environment
import android.provider.Settings
import android.view.View
import java.io.File
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.content.FileProvider
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  private var webView: WebView? = null
  private var keyboardOpen = false

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // Android draws apps under the status bar, the navigation bar and the keyboard (always, from
    // Android 15). Keep the page clear of them: with the keyboard open, it ends above it.
    val content = findViewById<View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout() or WindowInsetsCompat.Type.ime()
      )
      view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
      // The phone layout hides its bar at the bottom while the keyboard is open (phone.ts).
      val keyboard = insets.isVisible(WindowInsetsCompat.Type.ime())
      if (keyboard != keyboardOpen) {
        keyboardOpen = keyboard
        tellKeyboard()
      }
      WindowInsetsCompat.CONSUMED
    }
  }

  // The interface reaches this as window.WrtierAndroid (app/src/android.ts).
  override fun onWebViewCreate(webView: WebView) {
    this.webView = webView
    webView.addJavascriptInterface(Bridge(), "WrtierAndroid")
  }

  private fun tellKeyboard() {
    webView?.evaluateJavascript("document.documentElement.classList.toggle('keyboard-open', $keyboardOpen)", null)
  }

  inner class Bridge {
    /** "All files access" (ADR 0006): the Vault is read and written with plain file paths. */
    @JavascriptInterface
    fun hasAllFilesAccess(): Boolean = Environment.isExternalStorageManager()

    /**
     * Android never asks for "All files access" itself: this opens its switch for the app (or
     * the list of apps, where a phone has no page for one app).
     */
    @JavascriptInterface
    fun openAllFilesAccessSettings() {
      runOnUiThread {
        try {
          startActivity(Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION, Uri.parse("package:$packageName")))
        } catch (e: Exception) {
          startActivity(Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION))
        }
      }
    }

    /** The app's own page in Settings, for "Allow restricted settings" on an app not from a store. */
    @JavascriptInterface
    fun openAppInfo() {
      runOnUiThread {
        startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:$packageName")))
      }
    }

    /** Where the folder browser starts: the phone's shared storage ("/storage/emulated/0"). */
    @JavascriptInterface
    fun sharedStorage(): String = Environment.getExternalStorageDirectory().absolutePath

    /** App updates (ADR 0014): the app may hand an APK to Android's installer ("Install unknown apps"). */
    @JavascriptInterface
    fun canInstallApps(): Boolean = packageManager.canRequestPackageInstalls()

    /** Opens the "Install unknown apps" switch for this app. */
    @JavascriptInterface
    fun openInstallAppsSettings() {
      runOnUiThread {
        startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:$packageName")))
      }
    }

    /**
     * Opens Android's installer on a downloaded update (an APK in the app's cache), where the
     * Author taps Install. Android refuses it unless it's signed with the app's own release key.
     */
    @JavascriptInterface
    fun installApk(path: String) {
      val apk = File(path)
      if (!apk.canonicalPath.startsWith(cacheDir.canonicalPath + File.separator)) return
      val uri = FileProvider.getUriForFile(this@MainActivity, "$packageName.fileprovider", apk)
      runOnUiThread {
        startActivity(
          Intent(Intent.ACTION_VIEW)
            .setDataAndType(uri, "application/vnd.android.package-archive")
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        )
      }
    }
  }
}
