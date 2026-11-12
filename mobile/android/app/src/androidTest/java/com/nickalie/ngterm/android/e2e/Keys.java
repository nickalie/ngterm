package com.nickalie.ngterm.android.e2e;

import android.content.Context;
import android.os.Build;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.widget.Button;
import android.widget.LinearLayout;

/** The test keyboards' key layout: rows of buttons, each row filling the width, each key found by its description. */
final class Keys {
    // the least room left under the keys: the navigation bar's, where the system draws its buttons for input methods
    // (hide the keyboard, switch it), which a tap on a key there would hit instead
    private static final int NAVIGATION_FLOOR_DP = 56;

    private final Context context;
    private final LinearLayout rows;
    private LinearLayout row;

    Keys(Context context) {
        this.context = context;
        rows = new LinearLayout(context);
        rows.setOrientation(LinearLayout.VERTICAL);
    }

    /** Starts a new row below the last. */
    void row() {
        row = new LinearLayout(context);
        row.setOrientation(LinearLayout.HORIZONTAL);
        rows.addView(row, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.WRAP_CONTENT));
    }

    /** A key labeled {@code label}, described {@code description}, that runs {@code tap} when tapped. */
    void key(String label, String description, Runnable tap) {
        Button key = new Button(context);
        key.setText(label);
        key.setContentDescription(description);
        key.setAllCaps(false);
        key.setMinWidth(0);
        key.setMinimumWidth(0);
        key.setPadding(0, 0, 0, 0);
        key.setOnClickListener(view -> tap.run());
        row.addView(key, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f));
    }

    /** The keyboard, its keys above the navigation bar. */
    View view() {
        int floor = Math.round(NAVIGATION_FLOOR_DP * context.getResources().getDisplayMetrics().density);
        rows.setPadding(0, 0, 0, floor);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            rows.setOnApplyWindowInsetsListener((view, insets) -> {
                int bar = insets.getInsets(WindowInsets.Type.navigationBars()).bottom;
                view.setPadding(0, 0, 0, Math.max(bar, floor));
                return insets;
            });
        }
        return rows;
    }
}
