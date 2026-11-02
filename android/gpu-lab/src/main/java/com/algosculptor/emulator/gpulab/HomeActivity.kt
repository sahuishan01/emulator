package com.algosculptor.emulator.gpulab

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.content.res.Configuration
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.os.Bundle
import android.text.Editable
import android.text.TextWatcher
import android.text.format.DateUtils
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView

class HomeActivity : Activity() {
    companion object { private const val IMPORT = 40 }
    private val store get() = (application as EmulatorApplication).library
    private lateinit var ui: AppUi
    private lateinit var titles: LinearLayout
    private lateinit var banner: LinearLayout
    private lateinit var search: EditText
    private lateinit var importButton: Button
    private var entries = emptyList<LibraryEntry>()
    private val observer: (ImportStatus) -> Unit = { status ->
        importButton.isEnabled = !status.busy
        banner.removeAllViews()
        if (status.message.isNotEmpty()) {
            banner.addView(ui.text(status.message, 14f, if (status.error) Color.rgb(156, 42, 35) else ui.muted))
            banner.addView(ui.button(if (status.busy) "Cancel import" else "Dismiss") { if (status.busy) store.cancelImport() else store.dismissStatus() })
        }
        if (!status.busy) refresh()
    }
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState); ui = AppUi(this)
        val root = ui.column()
        val top = LinearLayout(this)
        top.addView(ui.label("EMULATOR"), LinearLayout.LayoutParams(0, -2, 1f))
        top.addView(ui.button("Settings") { startActivity(Intent(this, SettingsActivity::class.java)) })
        root.addView(top); ui.space(root, 22)
        root.addView(ui.text("Your library", 30f, bold = true)); ui.space(root, 6)
        root.addView(ui.text("Play included homebrew or import your own.", 15f, ui.muted)); ui.space(root, 18)
        importButton = ui.button("Import homebrew", true) {
            startActivityForResult(Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                addCategory(Intent.CATEGORY_OPENABLE); type = "*/*"
            }, IMPORT)
        }
        root.addView(importButton)
        ui.space(root, 10)
        root.addView(ui.text("Static .nro homebrew · up to 32 MiB per file. Commercial game loading is in development.", 12f, ui.muted))
        ui.space(root, 14)
        search = EditText(this).apply { hint = "Search library"; isSingleLine = true; setTextColor(ui.ink); setHintTextColor(ui.muted); textSize = 16f }
        root.addView(search)
        banner = ui.column(); root.addView(banner)
        titles = ui.column()
        val landscape = resources.configuration.orientation == Configuration.ORIENTATION_LANDSCAPE
        if (landscape) root.addView(titles)
        else root.addView(ScrollView(this).apply { addView(titles) }, LinearLayout.LayoutParams(-1, 0, 1f))
        root.addView(ui.label("DEVELOPMENT BUILD · HOMEBREW"))
        ui.install(if (landscape) ScrollView(this).apply { addView(root) } else root)
        search.addTextChangedListener(object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) { }
            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) { render() }
            override fun afterTextChanged(s: Editable?) { }
        })
        search.setText(savedInstanceState?.getString("query") ?: "")
    }
    override fun onStart() { super.onStart(); store.observe(observer); refresh() }
    override fun onStop() { store.unobserve(observer); super.onStop() }
    override fun onSaveInstanceState(outState: Bundle) { outState.putString("query", search.text.toString()); super.onSaveInstanceState(outState) }
    private fun refresh() { store.list { if (!isDestroyed && !isFinishing) { entries = it; render() } } }
    private fun render() {
        titles.removeAllViews()
        val visible = entries.filter { it.title.contains(search.text.toString(), ignoreCase = true) }
        if (visible.isEmpty()) { ui.space(titles, 24); titles.addView(ui.text(if (entries.isEmpty()) "Loading your library…" else "No matching homebrew.", 16f, ui.muted)); return }
        visible.forEach { entry ->
            ui.space(titles, 14)
            val card = ui.card()
            if (entry.included) {
                card.addView(object : View(this) {
                    private val paint = Paint()
                    init { contentDescription = "Catch preview: green square and red paddle" }
                    override fun onDraw(canvas: Canvas) {
                        super.onDraw(canvas); canvas.drawColor(Color.rgb(18, 25, 23))
                        paint.color = Color.rgb(80, 207, 117); canvas.drawRect(width * .64f, height * .25f, width * .70f, height * .43f, paint)
                        paint.color = Color.rgb(228, 99, 83); canvas.drawRect(width * .38f, height * .78f, width * .62f, height * .84f, paint)
                    }
                }, LinearLayout.LayoutParams(-1, ui.dp(118)))
                ui.space(card, 16)
            }
            card.addView(ui.label(if (entry.included) "INCLUDED HOMEBREW" else "IMPORTED · STATIC NRO"))
            ui.space(card, 6); card.addView(ui.text(entry.title, 23f, bold = true)); ui.space(card, 6)
            val missing = !entry.included && store.file(entry) == null
            card.addView(ui.text(if (missing) "File missing. Remove this entry and import it again." else if (entry.included) "Move the paddle. Catch the falling square." else if (entry.result.isEmpty()) "Ready to try · compatibility untested" else entry.result, 14f, ui.muted))
            if (entry.lastPlayed > 0) { ui.space(card, 8); card.addView(ui.text("Last played ${DateUtils.getRelativeTimeSpanString(entry.lastPlayed, System.currentTimeMillis(), DateUtils.MINUTE_IN_MILLIS)}", 12f, ui.muted)) }
            ui.space(card, 14)
            val actions = LinearLayout(this)
            actions.addView(ui.button(if (entry.included) "Play Catch" else "Run homebrew", true) { launch(entry) }.apply { isEnabled = !missing }, LinearLayout.LayoutParams(0, -2, 1f))
            actions.addView(ui.button("Details") { details(entry) })
            card.addView(actions); titles.addView(card)
        }
        ui.space(titles, 18)
    }
    private fun launch(entry: LibraryEntry) {
        startActivity(Intent(this, if (entry.included) DemoActivity::class.java else ProgramActivity::class.java).apply {
            putExtra("entryId", entry.id); putExtra("playerMode", true)
        })
    }
    private fun details(entry: LibraryEntry) {
        val text = if (entry.included) "Included homebrew. Drag the screen, hold Left/Right, or use a D-pad.\n\nSessions played: ${entry.launches}" else
            "NRO0 · ${entry.bytes} bytes\nSessions started: ${entry.launches}\n\nThis build runs a limited static homebrew subset. A valid file may still require instructions or services that are not implemented."
        val dialog = AlertDialog.Builder(this).setTitle(entry.title).setMessage(text).setPositiveButton("Close", null)
        if (!entry.included) dialog.setNeutralButton("Remove") { _, _ ->
            AlertDialog.Builder(this).setTitle("Remove ${entry.title}?").setMessage("Remove this homebrew and its imported copy from your library?")
                .setNegativeButton("Cancel", null).setPositiveButton("Remove") { _, _ -> store.remove(entry) }.show()
        }
        dialog.show()
    }
    @Deprecated("Activity result compatibility")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == IMPORT && resultCode == RESULT_OK) data?.data?.let { store.import(it) }
    }
}
