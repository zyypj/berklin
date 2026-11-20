package dev.zyypj.berklin

import android.Manifest
import android.annotation.SuppressLint
import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.PowerManager
import android.provider.Settings
import android.view.View
import android.view.ViewGroup
import android.view.WindowInsets
import android.view.WindowInsetsController
import android.webkit.PermissionRequest
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.FrameLayout
import android.widget.Toast
import android.window.OnBackInvokedCallback
import android.window.OnBackInvokedDispatcher
import org.json.JSONObject

class MainActivity : Activity(), RelayHub.Listener, Updater.Listener {
    private lateinit var webView: WebView
    private lateinit var assetServer: AppAssetServer
    private var backgroundColor = 0
    private var pageReady = false
    private var sessionToOpen: String? = null
    private var backCallback: Any? = null
    private var cameraRequestAwaitingPermission: PermissionRequest? = null
    private var installAfterUnknownSourcesGrant = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        backgroundColor = getColor(R.color.berklin_background)
        assetServer = AppAssetServer(assets)
        val root = FrameLayout(this)
        root.setBackgroundColor(backgroundColor)
        webView = createWebView()
        root.addView(webView, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        setContentView(root)
        configureSystemBars(root)
        registerBackHandling()
        handleLaunchIntent(intent, activityAlreadyRunning = false)
        requestNotificationPermission()
        RelayService.start(this)
        RelayHub.addListener(this)
        Updater.addListener(this)
        webView.loadUrl(UI_URL)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleLaunchIntent(intent, activityAlreadyRunning = true)
    }

    override fun onResume() {
        super.onResume()
        if (installAfterUnknownSourcesGrant && packageManager.canRequestPackageInstalls()) {
            installAfterUnknownSourcesGrant = false
            startUpdateInstall()
        }
    }

    override fun onDestroy() {
        RelayHub.removeListener(this)
        Updater.removeListener(this)
        cameraRequestAwaitingPermission?.deny()
        cameraRequestAwaitingPermission = null
        unregisterBackHandling()
        webView.removeJavascriptInterface(BRIDGE_NAME)
        webView.destroy()
        super.onDestroy()
    }

    @Deprecated("Used only below API 33; newer releases dispatch through OnBackInvokedCallback.")
    override fun onBackPressed() {
        handleBack()
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode != CAMERA_PERMISSION_REQUEST) return
        val pending = cameraRequestAwaitingPermission ?: return
        cameraRequestAwaitingPermission = null
        val granted = grantResults.isNotEmpty() && grantResults[0] == PackageManager.PERMISSION_GRANTED
        if (granted) pending.grant(arrayOf(PermissionRequest.RESOURCE_VIDEO_CAPTURE)) else pending.deny()
    }

    override fun onAgentMessage(json: String) {
        callUi("onMessage", JSONObject.quote(json))
    }

    override fun onRelayStatus(statusJson: String) {
        callUi("onStatus", JSONObject.quote(statusJson))
    }

    override fun onUpdateState(stateJson: String) {
        callUi("onUpdate", JSONObject.quote(stateJson))
    }

    fun applyPairingLink(link: String?, reloadPage: Boolean): Boolean {
        val pairing = Pairing.fromLink(link) ?: return false
        val store = PairingStore(this)
        val previous = store.load()
        val samePc = previous != null && previous.key.contentEquals(pairing.key)
        store.save(pairing)
        if (!samePc) {
            RelayHub.clearOutbox()
            ClaudeNotifier(this).cancelAllClaudeNotifications()
        }
        RelayService.reload(applicationContext)
        if (reloadPage) runOnUiThread { reloadUi() }
        return true
    }

    fun forgetPairing() {
        PairingStore(this).clear()
        RelayHub.clearOutbox()
        RelayService.stop(applicationContext)
        ClaudeNotifier(this).cancelAllClaudeNotifications()
        RelayHub.publishStatus(RelayState.OFFLINE, false)
    }

    fun startUpdateInstall() {
        if (!packageManager.canRequestPackageInstalls()) installAfterUnknownSourcesGrant = true
        if (!Updater.install(this)) {
            installAfterUnknownSourcesGrant = false
            Updater.reportInstallFailure("Nenhuma atualização baixada")
        }
    }

    fun openBatteryOptimizationSettings() {
        val powerManager = getSystemService(PowerManager::class.java)
        val alreadyIgnoring = powerManager?.isIgnoringBatteryOptimizations(packageName) == true
        val requestIntent = Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, Uri.parse("package:$packageName"))
        val listIntent = Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)
        val detailsIntent = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:$packageName"))
        val candidates = if (alreadyIgnoring) listOf(listIntent, detailsIntent) else listOf(requestIntent, listIntent, detailsIntent)
        for (candidate in candidates) {
            try {
                startActivity(candidate)
                return
            } catch (missing: ActivityNotFoundException) {
                continue
            } catch (denied: SecurityException) {
                continue
            }
        }
    }

    @SuppressLint("SetJavaScriptEnabled")
    private fun createWebView(): WebView {
        if (applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0) WebView.setWebContentsDebuggingEnabled(true)
        return WebView(this).apply {
            setBackgroundColor(backgroundColor)
            overScrollMode = View.OVER_SCROLL_NEVER
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            settings.allowFileAccess = false
            settings.allowContentAccess = false
            settings.mediaPlaybackRequiresUserGesture = false
            settings.textZoom = 100
            addJavascriptInterface(BerklinBridge(this@MainActivity), BRIDGE_NAME)
            webViewClient = BerklinWebViewClient()
            webChromeClient = BerklinChromeClient()
        }
    }

    @Suppress("DEPRECATION")
    private fun configureSystemBars(root: FrameLayout) {
        window.statusBarColor = backgroundColor
        window.navigationBarColor = backgroundColor
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            window.setDecorFitsSystemWindows(false)
            window.insetsController?.setSystemBarsAppearance(
                0,
                WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS or WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS,
            )
            root.setOnApplyWindowInsetsListener { view, insets ->
                val visibleTypes = WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout() or WindowInsets.Type.ime()
                val padding = insets.getInsets(visibleTypes)
                view.setPadding(padding.left, padding.top, padding.right, padding.bottom)
                WindowInsets.CONSUMED
            }
        }
    }

    private fun registerBackHandling() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            val callback = OnBackInvokedCallback { handleBack() }
            onBackInvokedDispatcher.registerOnBackInvokedCallback(OnBackInvokedDispatcher.PRIORITY_DEFAULT, callback)
            backCallback = callback
        }
    }

    private fun unregisterBackHandling() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            (backCallback as? OnBackInvokedCallback)?.let(onBackInvokedDispatcher::unregisterOnBackInvokedCallback)
            backCallback = null
        }
    }

    private fun handleBack() {
        if (!pageReady) {
            finish()
            return
        }
        webView.evaluateJavascript(BACK_SCRIPT) { result ->
            if (result != "true") finish()
        }
    }

    private fun handleLaunchIntent(intent: Intent?, activityAlreadyRunning: Boolean) {
        if (intent == null) return
        val data = intent.data
        if (intent.action == Intent.ACTION_VIEW && Pairing.isPairingLink(data)) {
            val paired = applyPairingLink(data.toString(), reloadPage = activityAlreadyRunning)
            Toast.makeText(this, if (paired) "Pareado com sucesso" else "Link de pareamento inválido", Toast.LENGTH_SHORT).show()
            return
        }
        if (intent.action == ACTION_INSTALL_UPDATE) {
            startUpdateInstall()
            return
        }
        val sid = intent.getStringExtra(EXTRA_SID)
        if (!sid.isNullOrEmpty()) {
            sessionToOpen = sid
            if (pageReady) deliverSessionToOpen()
        }
    }

    private fun requestNotificationPermission() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        if (checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED) return
        requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), NOTIFICATION_PERMISSION_REQUEST)
    }

    private fun handleCameraRequest(request: PermissionRequest) {
        val wantsVideo = PermissionRequest.RESOURCE_VIDEO_CAPTURE in request.resources
        if (!wantsVideo || request.origin.toString().trimEnd('/') != AppAssetServer.ORIGIN) {
            request.deny()
            return
        }
        if (checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) {
            request.grant(arrayOf(PermissionRequest.RESOURCE_VIDEO_CAPTURE))
            return
        }
        cameraRequestAwaitingPermission?.deny()
        cameraRequestAwaitingPermission = request
        requestPermissions(arrayOf(Manifest.permission.CAMERA), CAMERA_PERMISSION_REQUEST)
    }

    private fun reloadUi() {
        pageReady = false
        webView.loadUrl(UI_URL)
    }

    private fun deliverSessionToOpen() {
        val sid = sessionToOpen ?: return
        sessionToOpen = null
        callUi("onOpen", JSONObject.quote(sid))
    }

    private fun callUi(function: String, quotedArgument: String) {
        val script = "(function(b){if(b&&b.$function)b.$function($quotedArgument)})(window.__berklin)"
        runOnUiThread {
            if (pageReady && !isDestroyed) webView.evaluateJavascript(script, null)
        }
    }

    private inner class BerklinChromeClient : WebChromeClient() {
        override fun onPermissionRequest(request: PermissionRequest) {
            runOnUiThread { handleCameraRequest(request) }
        }

        override fun onPermissionRequestCanceled(request: PermissionRequest) {
            if (cameraRequestAwaitingPermission === request) cameraRequestAwaitingPermission = null
        }
    }

    private inner class BerklinWebViewClient : WebViewClient() {
        override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse? =
            assetServer.respond(request)

        override fun onPageFinished(view: WebView, url: String) {
            if (!url.startsWith(UI_ORIGIN)) return
            pageReady = true
            callUi("onStatus", JSONObject.quote(RelayHub.statusJson))
            Updater.state?.let { callUi("onUpdate", JSONObject.quote(it.toJson())) }
            deliverSessionToOpen()
        }

        override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean {
            val uri = request.url
            if (assetServer.serves(uri)) return false
            if (Pairing.isPairingLink(uri)) {
                applyPairingLink(uri.toString(), reloadPage = true)
                return true
            }
            try {
                startActivity(Intent(Intent.ACTION_VIEW, uri).addCategory(Intent.CATEGORY_BROWSABLE))
            } catch (missing: ActivityNotFoundException) {
                return true
            }
            return true
        }
    }

    companion object {
        const val ACTION_OPEN_SESSION = "dev.zyypj.berklin.action.OPEN_SESSION"
        const val ACTION_INSTALL_UPDATE = "dev.zyypj.berklin.action.INSTALL_UPDATE"
        const val EXTRA_SID = "sid"
        private const val UI_ORIGIN = AppAssetServer.ORIGIN + "/ui/"
        private const val UI_URL = UI_ORIGIN + "index.html"
        private const val BRIDGE_NAME = "BerklinNative"
        private const val NOTIFICATION_PERMISSION_REQUEST = 1
        private const val CAMERA_PERMISSION_REQUEST = 2
        private const val BACK_SCRIPT =
            "(function(){try{var b=window.__berklin;return !!(b&&b.onBack&&String(b.onBack())==='true')}catch(e){return false}})()"
    }
}
