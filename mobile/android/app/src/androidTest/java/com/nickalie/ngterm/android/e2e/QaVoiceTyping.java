package com.nickalie.ngterm.android.e2e;

import android.content.ComponentName;
import android.inputmethodservice.InputMethodService;
import android.view.View;
import androidx.annotation.RequiresApi;

/**
 * A stand-in for Google voice typing: another input method {@link QaKeyboard} switches to, whose only key goes back to
 * the keyboard ({@link #BACK}), as voice typing does once dictation ends. While it is up, the terminal's
 * input method is this one, so the system binds the input method again on the way there and on the way back.
 */
@RequiresApi(28)
public class QaVoiceTyping extends InputMethodService {
    /** The content description of its key back to the keyboard. */
    public static final String BACK = "qa-voice-back";

    /** This input method's id, for {@code ime enable} and {@code ime set}. */
    public static String id(String packageName) {
        return new ComponentName(packageName, QaVoiceTyping.class.getName()).flattenToShortString();
    }

    @Override
    public boolean onEvaluateInputViewShown() {
        return true;
    }

    @Override
    public View onCreateInputView() {
        Keys keys = new Keys(this);
        keys.row();
        // by its id: switchToPreviousInputMethod finds no earlier input method for a test APK's keyboard; either way the
        // system binds the keyboard again, which is what the test is about
        keys.key("back to keyboard", BACK, () -> switchInputMethod(QaKeyboard.id(getPackageName())));
        return keys.view();
    }
}
