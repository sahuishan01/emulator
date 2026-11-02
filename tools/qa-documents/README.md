# Test-only document provider

This separate Android APK supplies only our owned NRO fixtures, invalid files,
an over-limit stream and deliberately slow pipes. It exercises actual file-picker
grants, cancellation, copying and activity recreation. It is never a dependency
of the main application and must not be included in release assets.

Build both apps with:

```sh
./android/gradlew -p android :gpu-lab:assembleDebug :gpu-lab:lintDebug :qa-documents:assembleDebug
```

The provider assets are generated from the repository's own independently
assembled `hello.nro` and `ipc-clone.nro` by `tools/generate_qa_documents.py`.
Generated assets and APKs stay under ignored build directories.

`tools/qa_production_app.py` requires the explicitly owned `emulator-5580`,
a clean test library and default player preferences. It does not clear application
data itself. Never use `pm clear` against a user's device. Retain an authentic
previous-workload report separately if testing cache invalidation afterward.

```sh
python3 tools/qa_production_app.py \
  --apk android/gpu-lab/build/outputs/apk/debug/gpu-lab-debug.apk \
  --provider-apk tools/qa-documents/build/outputs/apk/debug/qa-documents-debug.apk \
  --output reports/android-qa/v0.20.0-dev/app-final
```

The runner saves fresh XML, screenshots, catalog snapshots, logcat, crash buffers
and exit history. It uninstalls the provider during the private-copy persistence
check, restores the tested player defaults, and leaves generated evidence ignored.
Follow it with `qa_homebrew.py --hold-controls`, then `qa_android_batch.py` for the
support diagnostics/export/cache-recovery flow. Do not run UI runners concurrently.
