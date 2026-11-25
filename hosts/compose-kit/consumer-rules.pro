# Keep JNI native bridge entry points intact under R8/ProGuard.
-keep class dev.nve.compose.NveJniBridge { native <methods>; }
-keep class dev.nve.compose.NveNativeBridge { *; }
