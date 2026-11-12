# Ghostty restores ZDOTDIR, reads the user's .zshenv and installs its deferred hooks.
'builtin' 'source' "$NGTERM_WSL_GHOSTTY/shell-integration/zsh/.zshenv"
'builtin' 'unset' NGTERM_WSL_GHOSTTY

__ngterm_wsl_pwd() {
    local ngterm_status=$?
    case "$PWD" in
        *[$'\001'-$'\037'$'\177']*) return "$ngterm_status" ;;
    esac
    builtin printf '\e]7;kitty-shell-cwd://localhost%s\a' "$PWD"
    return "$ngterm_status"
}
'builtin' 'typeset' -ga precmd_functions
precmd_functions+=(__ngterm_wsl_pwd)
