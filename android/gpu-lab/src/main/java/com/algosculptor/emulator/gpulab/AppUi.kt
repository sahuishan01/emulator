package com.algosculptor.emulator.gpulab

import android.app.Activity
import android.content.Context
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.view.View
import android.view.WindowInsets
import android.view.WindowInsetsController
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView

object AppSettings {
    fun prefs(context: Context) = context.getSharedPreferences("app-settings", Context.MODE_PRIVATE)
    fun repeatMs(context: Context): Long = prefs(context).getInt("repeatMs", 80).coerceIn(50, 120).toLong()
    fun showStats(context: Context) = prefs(context).getBoolean("showStats", false)
    fun keepAwake(context: Context) = prefs(context).getBoolean("keepAwake", true)
}
class AppUi(private val activity: Activity) {
    val background = Color.rgb(245, 246, 242)
    val ink = Color.rgb(24, 35, 31)
    val muted = Color.rgb(93, 110, 101)
    val accent = Color.rgb(44, 92, 66)
    fun dp(value: Int) = (value * activity.resources.displayMetrics.density).toInt()
    fun text(value: String, size: Float = 16f, color: Int = ink, bold: Boolean = false): TextView = TextView(activity).apply {
        text = value; textSize = size; setTextColor(color)
        if (bold) typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
    }
    fun label(value: String): TextView = text(value, 11f, muted).apply { typeface = Typeface.MONOSPACE; letterSpacing = .09f }
    fun column(): LinearLayout = LinearLayout(activity).apply { orientation = LinearLayout.VERTICAL }
    fun button(value: String, primary: Boolean = false, action: () -> Unit): Button = Button(activity).apply {
        text = value; isAllCaps = false; textSize = 15f; minHeight = dp(48)
        setTextColor(if (primary) Color.WHITE else accent)
        backgroundTintList = android.content.res.ColorStateList.valueOf(if (primary) accent else Color.rgb(228, 235, 227))
        setOnClickListener { action() }
    }
    fun card(): LinearLayout = column().apply {
        setPadding(dp(20), dp(20), dp(20), dp(20))
        background = GradientDrawable().apply { setColor(Color.WHITE); cornerRadius = dp(14).toFloat(); setStroke(dp(1), Color.rgb(217, 225, 216)) }
    }
    fun space(parent: LinearLayout, height: Int) { parent.addView(View(activity), LinearLayout.LayoutParams(1, dp(height))) }
    fun install(root: View) {
        root.setBackgroundColor(background)
        val padding = dp(20)
        root.setPadding(padding, padding, padding, padding)
        root.setOnApplyWindowInsetsListener { v, insets ->
            val bars = insets.getInsets(WindowInsets.Type.systemBars())
            v.setPadding(padding + bars.left, padding + bars.top, padding + bars.right, padding + bars.bottom); insets
        }
        activity.setContentView(root)
        activity.window.statusBarColor = background
        activity.window.navigationBarColor = background
        activity.window.insetsController?.setSystemBarsAppearance(
            WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS or WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS,
            WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS or WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS,
        )
    }
}
