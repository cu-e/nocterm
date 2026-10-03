function __nocterm_prompt --on-event fish_prompt
  printf '\e]7;file://localhost%s\a\e]133;A\a' (string escape --style=url $PWD)
end
function __nocterm_preexec --on-event fish_preexec
  printf '\e]133;C\a'
end
