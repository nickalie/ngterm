package com.nickalie.ngterm.android

import androidx.compose.ui.test.SemanticsNodeInteractionsProvider
import androidx.compose.ui.test.junit4.ComposeTestRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText

// the stores answer on their own threads, which the compose rule's idling doesn't see: wait for what they bring
private const val STORE_WAIT_MS = 10_000L

/** Waits until a node shows [text] (a substring when [substring]), as the stores' data arrives. */
fun <T> T.awaitText(
    text: String,
    substring: Boolean = false,
) where T : ComposeTestRule, T : SemanticsNodeInteractionsProvider {
    waitUntil(STORE_WAIT_MS) {
        onAllNodesWithText(text, substring = substring).fetchSemanticsNodes().isNotEmpty()
    }
}

/** Waits until [condition] holds, as the stores' threads bring it about. */
fun ComposeTestRule.awaitStore(condition: () -> Boolean) = waitUntil(STORE_WAIT_MS, condition)

/** Waits until a node is described as [description] (a status badge reads as its word). */
fun <T> T.awaitDescription(description: String) where T : ComposeTestRule, T : SemanticsNodeInteractionsProvider {
    waitUntil(STORE_WAIT_MS) {
        onAllNodesWithContentDescription(description).fetchSemanticsNodes().isNotEmpty()
    }
}
