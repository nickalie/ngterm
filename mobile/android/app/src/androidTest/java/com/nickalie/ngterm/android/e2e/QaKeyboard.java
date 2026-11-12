package com.nickalie.ngterm.android.e2e;

import android.content.ComponentName;
import android.inputmethodservice.InputMethodService;
import android.view.KeyEvent;
import android.view.View;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import androidx.annotation.RequiresApi;

/**
 * A soft keyboard the instrumented e2e tests type with, as a person taps one: its keys are views a test finds by their
 * content description ({@link #keyDescription}) and taps on the screen. It types as the common keyboards do: a word is
 * composed letter by letter ({@code setComposingText}) on the connection the system handed it
 * ({@code getCurrentInputConnection}), a space commits it with the space, Enter finishes it and sends the Enter key,
 * and a word in progress as the keyboard goes is finished there. Its mic key switches to {@link QaVoiceTyping}, as
 * Gboard's mic key switches to Google voice typing, which then switches back to it.
 *
 * <p>Plain Java: input methods run in the test APK's own process, which has no Kotlin runtime (the app's APK carries
 * it). Enabled only by the test that uses it ({@code ime enable}), from the test APK's manifest.
 */
// switchInputMethod and switchToPreviousInputMethod came in Android 9
@RequiresApi(28)
public class QaKeyboard extends InputMethodService {
    /** The mic key's name in {@link #keyDescription}. */
    public static final String VOICE = "voice";

    /** The space bar's name in {@link #keyDescription}. */
    public static final String SPACE = "space";

    /** The Enter key's name in {@link #keyDescription}. */
    public static final String ENTER = "enter";

    private static final String[] ROWS = {"1234567890", "qwertyuiop", "asdfghjkl-", "zxcvbnm./+"};

    private final StringBuilder word = new StringBuilder();

    /** The content description of the key that types {@code name} (a character, or VOICE, SPACE, ENTER). */
    public static String keyDescription(String name) {
        return "qa-key-" + name;
    }

    /** This keyboard's input method id, for {@code ime enable} and {@code ime set}. */
    public static String id(String packageName) {
        return new ComponentName(packageName, QaKeyboard.class.getName()).flattenToShortString();
    }

    @Override
    public boolean onEvaluateInputViewShown() {
        return true;
    }

    @Override
    public View onCreateInputView() {
        Keys keys = new Keys(this);
        for (String row : ROWS) {
            keys.row();
            for (char key : row.toCharArray()) {
                keys.key(String.valueOf(key), keyDescription(String.valueOf(key)), () -> type(key));
            }
        }
        keys.row();
        keys.key("mic", keyDescription(VOICE), () -> switchInputMethod(QaVoiceTyping.id(getPackageName())));
        keys.key("space", keyDescription(SPACE), this::space);
        keys.key("enter", keyDescription(ENTER), this::enter);
        return keys.view();
    }

    @Override
    public void onStartInput(EditorInfo attribute, boolean restarting) {
        super.onStartInput(attribute, restarting);
        word.setLength(0);
    }

    @Override
    public void onFinishInput() {
        // a word in progress is the editor's as the keyboard goes, as the common keyboards leave it
        InputConnection connection = getCurrentInputConnection();
        if (word.length() > 0 && connection != null) connection.finishComposingText();
        word.setLength(0);
        super.onFinishInput();
    }

    private void type(char key) {
        word.append(key);
        InputConnection connection = getCurrentInputConnection();
        if (connection != null) connection.setComposingText(word, 1);
    }

    private void space() {
        InputConnection connection = getCurrentInputConnection();
        if (connection != null) connection.commitText(word + " ", 1);
        word.setLength(0);
    }

    private void enter() {
        InputConnection connection = getCurrentInputConnection();
        if (connection == null) return;
        if (word.length() > 0) connection.finishComposingText();
        word.setLength(0);
        sendDownUpKeyEvents(KeyEvent.KEYCODE_ENTER);
    }
}
