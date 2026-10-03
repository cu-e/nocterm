if [[ -n "$NOCTERM_ORIGINAL_ZDOTDIR" ]]; then
  ZDOTDIR="$NOCTERM_ORIGINAL_ZDOTDIR"
else
  unset ZDOTDIR
fi
[[ -f "${ZDOTDIR:-$HOME}/.zshrc" ]] && source "${ZDOTDIR:-$HOME}/.zshrc"
autoload -Uz add-zsh-hook
__nocterm_prompt() {
  local p="$PWD"; p="${p//%/%25}"; p="${p// /%20}"
  printf '\033]7;file://localhost%s\007\033]133;A\007' "$p"
}
__nocterm_preexec() { printf '\033]133;C\007'; }
add-zsh-hook precmd __nocterm_prompt
add-zsh-hook preexec __nocterm_preexec
PROMPT+=$'%{\e]133;B\a%}'
