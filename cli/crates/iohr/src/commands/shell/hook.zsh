# iohr shell hook zsh: `? question` at the prompt asks your own AI agent (PRD 0187).
# Add to ~/.zshrc:  eval "$(iohr shell hook zsh)"
#
# A line that is `?` or `??`, a space and then the question, on one line, is a question; a
# `?` inside a word or a glob such as `?.txt` is not one. The shell never runs the line and
# keeps it out of its history. `?? ...` follows up the last answer. The answer is AI text
# under your own subscription and says so; nothing in it is ever run. INORBIT_ASK=0 turns
# this off for a shell. The folder and the last command (with its exit status) are sent as
# context; a plain shell cannot see that command's output.
if [[ -o interactive && -z ${functions[_inorbit_accept]-} ]]; then
  typeset -g _iohr_ask_q _iohr_ask_pq _iohr_ask_pa
  typeset -g _iohr_ask_re='^[[:space:]]*\?\??[[:space:]]+[^[:space:]]'

  # The widget: a question line is kept, the shell is handed an empty line, and the answer
  # is printed by the precmd below, before the next prompt (a widget cannot print safely).
  _iohr_ask_accept() {
    if [[ ${INORBIT_ASK-} != 0 && $BUFFER != *$'\n'* && $BUFFER =~ $_iohr_ask_re ]]; then
      _iohr_ask_q=$BUFFER
      PREDISPLAY=$BUFFER  # the typed line stays on screen while the shell gets an empty one
      BUFFER=""
    fi
    zle _iohr_ask_prev_accept
  }

  _iohr_ask_precmd() {
    local rc=$?
    [[ -n $_iohr_ask_q ]] || return 0
    local line=$_iohr_ask_q
    _iohr_ask_q=
    local last=${history[$((HISTCMD-1))]:-}
    local follow=0 q
    setopt localoptions extendedglob
    line=${line##[[:space:]]#}
    if [[ $line == '??'* ]]; then follow=1; q=${line#\?\?}; else q=${line#\?}; fi
    q=${q##[[:space:]]#}
    if [[ ${INORBIT_TERM-} == 1 ]]; then
      # Inside the InOrbit app: it draws the answer card from this, as its own hook does:
      # ESC ] 7727 ; ask ; ROWS ; QUESTION ; FOLDER ; LAST COMMAND BEL, each text in base64.
      local rows=${INORBIT_ASK_ROWS:-6} x; local -a f
      (( rows >= 3 && rows <= 12 )) || rows=6
      for x in "$line" "$PWD" "${last[1,1024]}"; do
        f+=("$(print -rn -- "$x" | command base64 | command tr -d '\n')")
      done
      print -rn -- $'\e]7727;ask;'"$rows;${(j:;:)f}"$'\a'
      repeat $rows print
      return 0
    fi
    # __IOHR_BIN__ is the name of the binary that printed this hook (iohr, iohr-preview).
    local -a a=(command __IOHR_BIN__ ask --headless --card --cwd "$PWD")
    [[ -n $last ]] && a+=(--last-command "${last[1,1024]}" --exit-code "$rc")
    (( follow )) && [[ -n $_iohr_ask_pq ]] && a+=(--prior-question "$_iohr_ask_pq" --prior-answer "$_iohr_ask_pa")
    local answer
    # The card goes to the terminal on stderr; the plain answer is kept for `??`.
    if answer=$("${a[@]}" -- "$q"); then
      _iohr_ask_pq=$q _iohr_ask_pa=$answer
    else
      _iohr_ask_pq= _iohr_ask_pa=
    fi
  }

  autoload -Uz add-zsh-hook
  # First, so the exit status it reads is the last command's.
  add-zsh-hook precmd _iohr_ask_precmd
  precmd_functions=(_iohr_ask_precmd ${precmd_functions:#_iohr_ask_precmd})
  # Wired once: sourcing this again (a second `source ~/.zshrc`, or a plugin such as
  # zsh-autosuggestions that re-wraps widgets) would copy our own widget into
  # _iohr_ask_prev_accept, and accept-line would call itself until FUNCNEST. The functions
  # above are safe to redefine; only the wiring is guarded, by the widget that holds the
  # previous accept-line (a shell variable can be reset, the widget table is the truth).
  if (( ! ${+widgets[_iohr_ask_prev_accept]} )); then
    # Chain to whatever accept-line was before (the shell's own, or the user's widget).
    zle -A accept-line _iohr_ask_prev_accept
    zle -N accept-line _iohr_ask_accept
  fi
fi
