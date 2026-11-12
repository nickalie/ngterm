# Executed by /bin/sh inside the selected WSL distribution. Arguments, never source:
# Ghostty resources (Windows), ngterm resources (Windows), login/return/end, command, Linux cwd.
# Do not run a login sh here: the user's actual shell must read its startup files once.
ngterm_shell=${SHELL:-/bin/sh}
case "$5" in
    /*) CDPATH= cd "$5" 2>/dev/null || : ;;
esac
if [ "$3" = end ]; then
    exec /bin/sh -lc "$4"
fi
if [ "$3" = return ]; then
    eval "$4"
fi
ngterm_ghostty=$(wslpath -a -u "$1" 2>/dev/null) || ngterm_ghostty=
ngterm_own=$(wslpath -a -u "$2" 2>/dev/null) || ngterm_own=
if [ -n "$ngterm_ghostty" ] && [ -n "$ngterm_own" ]; then
    export NGTERM_WSL_GHOSTTY="$ngterm_ghostty"
    case "${ngterm_shell##*/}" in
        bash)
            if [ -r "$ngterm_ghostty/shell-integration/bash/ghostty.bash" ] &&
               [ -r "$ngterm_own/shell-integration/wsl/bash-env" ]; then
                [ -z "${ENV+x}" ] || export GHOSTTY_BASH_ENV="$ENV"
                export ENV="$ngterm_own/shell-integration/wsl/bash-env"
                export GHOSTTY_BASH_INJECT=1
                if [ -z "${HISTFILE+x}" ]; then
                    export HISTFILE="$HOME/.bash_history" GHOSTTY_BASH_UNEXPORT_HISTFILE=1
                fi
                exec "$ngterm_shell" --posix -il
            fi
            ;;
        zsh)
            if [ -r "$ngterm_ghostty/shell-integration/zsh/.zshenv" ] &&
               [ -r "$ngterm_own/shell-integration/wsl/zsh/.zshenv" ]; then
                [ -z "${ZDOTDIR+x}" ] || export GHOSTTY_ZSH_ZDOTDIR="$ZDOTDIR"
                export ZDOTDIR="$ngterm_own/shell-integration/wsl/zsh"
                exec "$ngterm_shell" -il
            fi
            ;;
    esac
fi
unset NGTERM_WSL_GHOSTTY
exec "$ngterm_shell" -l
