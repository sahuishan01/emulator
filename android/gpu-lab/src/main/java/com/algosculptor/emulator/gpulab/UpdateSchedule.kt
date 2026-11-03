package com.algosculptor.emulator.gpulab

import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.content.ComponentName
import android.content.Context

internal object UpdateSchedule {
    const val ID = 4107
    fun sync(context: Context) {
        val manager = (context.applicationContext as EmulatorApplication).updates
        val scheduler = context.getSystemService(JobScheduler::class.java)
        if (!manager.connected || !manager.prefs.getBoolean("automatic", true)) { scheduler.cancel(ID); manager.cancel(backgroundOnly = true); return }
        scheduler.schedule(JobInfo.Builder(ID, ComponentName(context, UpdateJobService::class.java))
            .setRequiredNetworkType(JobInfo.NETWORK_TYPE_ANY).setRequiresBatteryNotLow(true).setRequiresStorageNotLow(true)
            .setPersisted(true).setPeriodic(UpdateManager.INTERVAL, 60L * 60 * 1000).setBackoffCriteria(30L * 60 * 1000, JobInfo.BACKOFF_POLICY_EXPONENTIAL).build())
    }
}
class UpdateJobService : JobService() {
    private var running: JobParameters? = null
    override fun onStartJob(params: JobParameters): Boolean {
        running = params
        val started = (application as EmulatorApplication).updates.automatic { if (running === params) { running = null; jobFinished(params, false) } }
        if (!started) running = null
        return started
    }
    override fun onStopJob(params: JobParameters): Boolean { running = null; (application as EmulatorApplication).updates.cancel(backgroundOnly = true); return true }
}
