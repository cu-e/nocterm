# Ephemeral --rcfile: retain the user's ordinary interactive setup.
[[ -f "$HOME/.bashrc" ]] && source "$HOME/.bashrc"
__nocterm_prompt() {
  local p="$PWD"; p="${p//%/%25}"; p="${p// /%20}"
  printf '\033]7;file://localhost%s\007\033]133;A\007' "$p"
}
if [[ $(declare -p PROMPT_COMMAND 2>/dev/null) == 'declare -a '* ]]; then
  PROMPT_COMMAND+=(__nocterm_prompt)
else
  PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND; }__nocterm_prompt"
fi
PS1+='\[\e]133;B\a\]'
# Bash 4.4+ prints PS0 after accepting a command, before executing it.
PS0=$'\e]133;C\a'"$PS0"
