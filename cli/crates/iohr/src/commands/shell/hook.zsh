# iohr shell hook zsh: `? question` at the prompt asks your own AI agent (PRD 0187).
# Add to ~/.zshrc:  eval "$(iohr shell hook zsh)"
#
# A line that is `?` or `??`, a space and then the question, on one line, is a question; a
# `?` inside a word or a glob such as `?.txt` is not one. The shell never runs the line and
# keeps it out of its history. `?? ...` follows up the last answer. The answer is AI text
# under your own subscription and says so; nothing in it is ever run. INORBIT_ASK=0 turns
# this off for a shell. The folder and the last command (with its exit status) are sent as
# context; a plain shell cannot see that command's output. Under an answer, the Down arrow
# on an empty line (`[ask] menu_key`) opens a menu: Run puts the answer's command on the
# prompt line for you to read and run with Enter, Copy, Follow up, Explain, Open. Nothing
# in the menu runs a command.
#
# The mode of the answers (PRD 0197): manual (ask before each command), plan (read only, a plan
# and no command) or auto (run what your rules do not forbid, without asking). Shift+Tab on a
# `?` line (or a bare `?`) goes to the next one and says which; on any other line it does what
# it did before. The mode starts as `[ask] mode` / IOHR_ASK_MODE say, manual when they
# say nothing (empty for the older `deny` and `rules`, which stay the config's), and lasts for
# this shell.
#
# The right prompt shows "3 asks" in grey when asks wait for you (`iohr asks count`). The
# prompt only reads the number from ~/.inorbit/term/asks-count: it never waits for the
# network. When that file is more than 30 seconds old a background `asks count --cache`
# refreshes it (at most once in 30 seconds), and the next prompt shows the new number. An
# RPROMPT you set yourself is left alone; INORBIT_ASKS_PROMPT=0 turns the segment off.
if [[ -o interactive && -z ${functions[_inorbit_accept]-} ]]; then
  typeset -g _iohr_ask_q _iohr_ask_pq _iohr_ask_pa _iohr_ask_cmd _iohr_ask_link _iohr_ask_undo _iohr_ask_live
  typeset -gA _iohr_card_prev
  # The keys that open the menu, from `[ask] menu_key` (none when it is off).
  typeset -ga _iohr_card_keys=(__IOHR_MENU_KEYS__)
  typeset -g _iohr_ask_re='^[[:space:]]*\?\??[[:space:]]+[^[:space:]]'
  # The mode of the answers, set when the hook is printed (from `[ask] mode` and the
  # environment), then one word of the ring below, passed to every ask as `--mode`.
  typeset -g _iohr_ask_mode='__IOHR_ASK_MODE__'
  typeset -ga _iohr_ask_ring=(__IOHR_MODES__)
  typeset -gA _iohr_mode_prev
  typeset -g _iohr_ask_mode_re='^[[:space:]]*\?\??[[:space:]]*$'

  # The widget: a question line is kept, the shell is handed an empty line, and the answer
  # is printed by the precmd below, before the next prompt (a widget cannot print safely).
  _iohr_ask_accept() {
    _iohr_ask_live=  # any new line ends the card's menu
    if [[ ${INORBIT_ASK-} != 0 && $BUFFER != *$'\n'* && $BUFFER =~ $_iohr_ask_re ]]; then
      _iohr_ask_q=$BUFFER
      PREDISPLAY=$BUFFER  # the typed line stays on screen while the shell gets an empty one
      BUFFER=""
    fi
    zle _iohr_ask_prev_accept
  }

  # The menu under the answer. It is only ever a key on an empty line after an answer; with
  # text on the line, or no answer, the key does what it did before. The menu draws on the
  # terminal and prints at most one line, which goes to the buffer: `iohr` never runs it, the
  # line is only run when you press Enter (it is set in BUFFER and nowhere else).
  _iohr_card_action() {
    if [[ -z $_iohr_ask_live || -n $BUFFER ]]; then
      zle ${_iohr_card_prev[$KEYMAP]:-down-line-or-history}
      return
    fi
    local out
    zle -I
    out=$(command __IOHR_BIN__ ask --menu --cwd "$PWD" --command="$_iohr_ask_cmd" --link="$_iohr_ask_link" --undo="$_iohr_ask_undo" </dev/tty 2>/dev/tty)
    if [[ -n $out ]]; then
      BUFFER=$out
      CURSOR=${#BUFFER}
    fi
    zle reset-prompt
  }

  # Shift+Tab: the next mode of the ring, on a `?` line only. Everywhere else the key keeps
  # the widget it had (completion backwards, usually).
  _iohr_ask_cycle() {
    if [[ ${INORBIT_ASK-} == 0 || $BUFFER == *$'\n'* ]] ||
       ! [[ $BUFFER =~ $_iohr_ask_re || $BUFFER =~ $_iohr_ask_mode_re ]]; then
      zle ${_iohr_mode_prev[$KEYMAP]:-reverse-menu-complete}
      return
    fi
    local i next=1
    for (( i = 1; i <= ${#_iohr_ask_ring}; i++ )); do
      [[ ${_iohr_ask_ring[i]} == $_iohr_ask_mode ]] && next=$(( i % ${#_iohr_ask_ring} + 1 ))
    done
    _iohr_ask_mode=${_iohr_ask_ring[next]}
    zle -M "ask mode: $_iohr_ask_mode (shift+tab for the next)"
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
    local -a a=(command __IOHR_BIN__ ask --headless --hook --cwd "$PWD")
    [[ -n $_iohr_ask_mode ]] && a+=(--mode "$_iohr_ask_mode")
    [[ -n $last ]] && a+=(--last-command "${last[1,1024]}" --exit-code "$rc")
    (( follow )) && [[ -n $_iohr_ask_pq ]] && a+=(--prior-question "$_iohr_ask_pq" --prior-answer "$_iohr_ask_pa")
    local rec rs=$'\x1e' rest tail
    # The card goes to the terminal on stderr. The record on stdout is the answer (kept for
    # `??`), the command it suggests, the link it names and the permission rules this run saved
    # (what Undo takes back), separated by a 0x1e character.
    if rec=$("${a[@]}" -- "$q"); then
      rest=${rec#*$rs}
      _iohr_ask_pq=$q
      _iohr_ask_pa=${rec%%$rs*}
      _iohr_ask_cmd=${rest%%$rs*}
      tail=${rest#*$rs}
      _iohr_ask_link=${tail%%$rs*}
      if [[ $tail == *$rs* ]]; then _iohr_ask_undo=${tail#*$rs}; else _iohr_ask_undo=; fi
      _iohr_ask_live=1
    else
      _iohr_ask_pq= _iohr_ask_pa= _iohr_ask_cmd= _iohr_ask_link= _iohr_ask_undo= _iohr_ask_live=
    fi
  }

  # The asks segment. Reads one small file, never the network; the refresh is detached
  # (`&!`) so it is not a job of this shell and prints nothing.
  zmodload -F zsh/stat b:zstat 2>/dev/null
  zmodload zsh/datetime 2>/dev/null
  typeset -g _iohr_asks_rp _iohr_asks_try=0
  _iohr_asks_precmd() {
    [[ ${INORBIT_ASKS_PROMPT-} == 0 ]] && return 0
    local f=${INORBIT_ASKS_COUNT_FILE:-$HOME/.inorbit/term/asks-count}
    local n= now=${EPOCHSECONDS:-0} mt=0 plural=s
    local -a st
    if [[ -r $f ]]; then
      n=$(<$f) 2>/dev/null
      zstat -A st +mtime -- $f 2>/dev/null && mt=${st[1]}
    fi
    # Stale or missing: refresh in the background, at most once in 30 seconds.
    if (( now > 0 && now - mt > 30 && now - _iohr_asks_try > 30 )); then
      _iohr_asks_try=$now
      command __IOHR_BIN__ asks count --cache >/dev/null 2>&1 &!
    fi
    # Only a number is ever shown; anything else in the file is ignored.
    [[ $n == <1-> ]] || n=
    # An RPROMPT that is not ours is somebody's choice: leave it as it is.
    if [[ -z $RPROMPT || $RPROMPT == $_iohr_asks_rp ]]; then
      if [[ -n $n ]]; then
        [[ $n == 1 ]] && plural=
        _iohr_asks_rp="%F{8}$n ask$plural%f"
      else
        _iohr_asks_rp=
      fi
      RPROMPT=$_iohr_asks_rp
    fi
  }

  autoload -Uz add-zsh-hook
  add-zsh-hook precmd _iohr_asks_precmd
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
  # The menu key, once per keymap and key: the widget that held it is kept so the key keeps
  # its meaning anywhere else (the same guard against wrapping ourselves).
  if (( ${#_iohr_card_keys} && ! ${+widgets[_iohr_card_action]} )); then
    zle -N _iohr_card_action
    local _iohr_km _iohr_seq _iohr_cur
    for _iohr_km in emacs viins; do
      for _iohr_seq in $_iohr_card_keys; do
        if [[ -z ${_iohr_card_prev[$_iohr_km]-} ]]; then
          _iohr_cur=$(bindkey -M $_iohr_km -- "$_iohr_seq" 2>/dev/null)
          _iohr_cur=${_iohr_cur##* }
          [[ -n $_iohr_cur && $_iohr_cur != undefined-key && $_iohr_cur != _iohr_card_action ]] &&
            _iohr_card_prev[$_iohr_km]=$_iohr_cur
        fi
        bindkey -M $_iohr_km -- "$_iohr_seq" _iohr_card_action
      done
    done
    unset _iohr_km _iohr_seq _iohr_cur
  fi
  # The mode key (Shift+Tab, `^[[Z`), once per keymap, with the same guard.
  if (( ! ${+widgets[_iohr_ask_cycle]} )); then
    zle -N _iohr_ask_cycle
    local _iohr_km _iohr_cur
    for _iohr_km in emacs viins; do
      _iohr_cur=$(bindkey -M $_iohr_km -- '^[[Z' 2>/dev/null)
      _iohr_cur=${_iohr_cur##* }
      [[ -n $_iohr_cur && $_iohr_cur != undefined-key && $_iohr_cur != _iohr_ask_cycle ]] &&
        _iohr_mode_prev[$_iohr_km]=$_iohr_cur
      bindkey -M $_iohr_km -- '^[[Z' _iohr_ask_cycle
    done
    unset _iohr_km _iohr_cur
  fi
fi
