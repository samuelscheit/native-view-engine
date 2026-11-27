# JNI entry points of `libnve_android_shell` and the callbacks it invokes by name.
-keep class dev.nve.host.NativeRuntime { native <methods>; }
-keep interface dev.nve.host.NativeRuntime$TextMeasurer { *; }
