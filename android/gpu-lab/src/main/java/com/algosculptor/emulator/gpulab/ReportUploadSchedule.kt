package com.algosculptor.emulator.gpulab

import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.content.ComponentName
import android.content.Context

internal object ReportUploadSchedule {
    const val ID = 4108
    fun sync(context: Context) {
        val prefs = context.getSharedPreferences("report-uploads", Context.MODE_PRIVATE)
        val jobs = context.getSystemService(JobScheduler::class.java)
        if (!prefs.getBoolean("automatic", false) || prefs.getBoolean("blocked", false) || prefs.getInt("pending", 0) == 0 || !UpdateVault(context).exists()) { jobs.cancel(ID); return }
        jobs.schedule(JobInfo.Builder(ID, ComponentName(context, ReportUploadJobService::class.java))
            .setRequiredNetworkType(JobInfo.NETWORK_TYPE_ANY).setRequiresBatteryNotLow(true).setRequiresStorageNotLow(true).setPersisted(true)
            .setMinimumLatency(maxOf(10_000, maxOf(prefs.getLong("nextAttempt", 0), prefs.getLong("serverWaitUntil", 0)) - System.currentTimeMillis()))
            .setBackoffCriteria(60_000, JobInfo.BACKOFF_POLICY_EXPONENTIAL).build())
    }
}
class ReportUploadJobService : JobService() {
    private var running: JobParameters? = null
    override fun onStartJob(params: JobParameters): Boolean {
        running = params
        val started = (application as EmulatorApplication).reportUploads.runPending { if (running === params) { running = null; jobFinished(params, false) } }
        if (!started) running = null
        return started
    }
    override fun onStopJob(params: JobParameters): Boolean { running = null; (application as EmulatorApplication).reportUploads.stop(); return true }
}
