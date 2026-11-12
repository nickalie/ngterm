---
worth: yes
where: mobile/android/app/src/main/kotlin/com/nickalie/ngterm/android/ui/chat/PaneChats.kt:116
added: 2026-11-12
---
# the Android chat view toggle reverts while an earlier write lands

`PaneChats.setChatView` sets `chatViewState` at once and then writes the choice to the store. But the
`combine(sessionViews.views, unsaved)` collector in `init` replaces `chatViewState` on every store emission.
So when a session's Chat view is toggled twice in quick succession (off, then on), the first write (off) can
land after the second tap. The toggle then reads Off until the second write lands. Users see the toggle flicker
back for a moment. In tests it flakes
`SessionsRouteTest.attachFromTheMenuDetachAndChatPreferenceReachTheRealSessionState` at its third
`assertIsOn` (line 175). That test failed once in a `scripts/build-android.sh check` run on the
android-new-session-opens dispatch branch, whose diff doesn't touch this path, and passed on the rerun.

A likely fix: keep each pending choice as an overlay over what the store reads (like `unsaved`), tagged so that
only the latest write for a session clears it once it lands.
