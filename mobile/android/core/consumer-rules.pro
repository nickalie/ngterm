# JNA and the UniFFI bindings are reached by reflection from native code: keep them whole when an app minifies.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keep class com.nickalie.ngterm.core.ffi.** { *; }
-dontwarn java.awt.**
