# iohr shell hook zsh: `? question` at the prompt asks your own AI agent (PRD 0187).
# Add to ~/.zshrc:  eval "$(iohr shell hook zsh)"
#
# A line that is `?` or `??`, a space and then the question, on one line, is a question; a
# `?` inside a word or a glob such as `?.txt` is not one. The shell never runs the line and
# keeps it out of its history. `?? ...` follows up the last answer. The answer is AI text
# under your own subscription and says so; nothing in it is ever run. INORBIT_ASK=0 turns
# this off for a shell. The folder and the last command (with its exit status) are sent as
# context; a plain shell cannot see that command's output, so it is not sent, and the card
# says so. After a failed command, a question such as "why did that fail" offers, in the
# menu below, Re-run with output (shows the command and asks first; it runs once, stopped
# after 10 seconds) and Paste output; what you attach is sent with the next ask, with
# secrets redacted, and the card lists it. Under an answer, the Down arrow
# on an empty line (`[ask] menu_key`) opens a menu: Run puts the answer's command on the
# prompt line for you to read and run with Enter, Copy, Follow up, Explain, Open. Nothing
# in the menu runs a command without that yes. A question the menu picks (a next step,
# Explain, Allow) is asked at once, with no second Enter; Follow up leaves `?? ` on the line
# for you to finish. An empty Enter keeps the card, so the key still works; running a command
# ends it.
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
#
# Inside `iohr panel` (RFC 0212) the terminal has a chat box at the bottom. This hook tells it,
# with private terminal commands (OSC 7771, written to the terminal and taken out by the
# panel, so no other terminal ever sees them), when the prompt is ready, when a command runs
# and which answer mode is set. A bare `?` at an empty prompt hands the keys to the box (a
# second `?` there means `??`); the box hands a question back as the private key
# `ESC [ 27771 ~`, which reads it from a private file the panel named in IOHR_PANEL_CHAT_FILE
# and asks it at once, so one Enter sends it. Outside the panel, or with `[panel] chat = "off"`,
# none of this is bound or sent and `?` is typed as before.
if [[ -o interactive && -z ${functions[_inorbit_accept]-} ]]; then
  typeset -g _iohr_ask_q _iohr_ask_pq _iohr_ask_pa _iohr_ask_cmd _iohr_ask_link _iohr_ask_undo _iohr_ask_access _iohr_ask_steps _iohr_ask_live
  # The command that failed under the last ask, its exit status, and the output attached to it
  # from the menu (a private file and how it came: captured or pasted).
  typeset -g _iohr_ask_failcmd _iohr_ask_failrc _iohr_ask_outfile _iohr_ask_outkind
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
    # A command ends the card's menu; an empty Enter (or spaces) keeps it, so the Down key
    # still opens the menu on the prompt that comes back.
    [[ $BUFFER == *[![:space:]]* ]] && _iohr_ask_live=
    if [[ ${INORBIT_ASK-} != 0 && $BUFFER != *$'\n'* && $BUFFER =~ $_iohr_ask_re ]]; then
      _iohr_ask_q=$BUFFER
      PREDISPLAY=$BUFFER  # the typed line stays on screen while the shell gets an empty one
      BUFFER=""
    fi
    zle _iohr_ask_prev_accept
  }

  # The menu under the answer. It is only ever a key on an empty line after an answer; with
  # text on the line, or no answer, the key does what it did before. The menu draws on the
  # terminal and prints at most one line; `iohr` never runs it. Run's command (after the menu's
  # yes when it is risky) and a whole `?` or `??` question are handed to accept-line at once;
  # anything else (a bare `?? `, to finish) stays in the buffer.
  _iohr_card_action() {
    if [[ -z $_iohr_ask_live || -n $BUFFER ]]; then
      zle ${_iohr_card_prev[$KEYMAP]:-down-line-or-history}
      return
    fi
    local out rs=$'\x1e' to=
    local -a extra
    zle -I
    # The folder Io needs a yes for and the next steps ride along: Allow and the steps need them.
    [[ -n $_iohr_ask_access ]] && extra+=(--access="$_iohr_ask_access")
    [[ -n $_iohr_ask_steps ]] && extra+=(--step="$_iohr_ask_steps")
    extra+=(--asked="$_iohr_ask_pq")
    # A private file for the output the menu may attach (removed when nothing comes of it).
    if [[ -n $_iohr_ask_failcmd ]]; then
      to=$(umask 077; command mktemp "${TMPDIR:-/tmp}/iohr-ask-output.XXXXXX" 2>/dev/null) &&
        extra+=(--failed-command "$_iohr_ask_failcmd" --failed-exit "$_iohr_ask_failrc" --output-to "$to")
    fi
    out=$(command __IOHR_BIN__ ask --menu --cwd "$PWD" --command="$_iohr_ask_cmd" --link="$_iohr_ask_link" --undo="$_iohr_ask_undo" "${extra[@]}" </dev/tty 2>/dev/tty)
    # Run came back as `command` 0x1e run: the command is the person's choice (the menu asked
    # first when it must), so it goes through accept-line like a line they typed.
    if [[ $out == *${rs}run ]]; then
      out=${out%${rs}run}
      [[ -n $to ]] && command rm -f -- "$to"
      if [[ -n $out && $out != *$'\n'* ]]; then
        BUFFER=$out
        CURSOR=${#BUFFER}
        zle accept-line
        return
      fi
      out=
    elif [[ -n $to ]]; then
      # Attached output came back as `? question` 0x1e kind; the file is read by the next ask.
      if [[ $out == *$rs* && -s $to ]]; then
        [[ -n $_iohr_ask_outfile ]] && command rm -f -- "$_iohr_ask_outfile"
        _iohr_ask_outfile=$to _iohr_ask_outkind=${out#*$rs}
        out=${out%%$rs*}
      else
        command rm -f -- "$to"
        out=${out%%$rs*}
      fi
    fi
    if [[ -n $out ]]; then
      BUFFER=$out
      CURSOR=${#BUFFER}
      # A chosen `?` or `??` question (a step, Explain, Allow, an attached output) is asked at
      # once: the line is the one this hook already takes for a question, and no shell ever
      # sees it. `zle accept-line` is our own widget (it empties the buffer and chains to the
      # shell's), so this cannot loop. Anything else stays in the buffer for you to finish or
      # run with Enter: a bare `?? ` (Follow up), `iohr asks list`.
      if [[ ${INORBIT_ASK-} != 0 && $BUFFER != *$'\n'* && $BUFFER =~ $_iohr_ask_re ]]; then
        zle accept-line
        return
      fi
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
    _iohr_chat_osc "mode;$_iohr_ask_mode"
    zle -M "ask mode: $_iohr_ask_mode (shift+tab for the next)"
  }

  # One report to the panel's chat box (RFC 0212), only inside a live panel that has a box:
  # the panel names its private question file in IOHR_PANEL_CHAT_FILE (not set when
  # `[panel] chat = "off"`) and IOHR_PANEL is its process id. Written to the terminal, where
  # the panel takes it out of the output; the shell prints nothing.
  _iohr_chat_osc() {
    [[ -n ${IOHR_PANEL_CHAT_FILE-} && -n ${IOHR_PANEL-} ]] && kill -0 $IOHR_PANEL 2>/dev/null || return 0
    printf '\e]7771;chat;%s\a' "$1" >/dev/tty 2>/dev/null
    return 0
  }
  # The prompt is ready: the answer mode now and the modes Shift+Tab goes through, so the box
  # never works them out itself.
  _iohr_chat_idle() { _iohr_chat_osc "idle;$_iohr_ask_mode;${(j:,:)_iohr_ask_ring}"; }
  # A command is about to run: the box gives the keys back.
  _iohr_chat_busy() { _iohr_chat_osc busy; }

  # `?` on an empty line, in a panel with a chat box: the box takes the keys (the panel sees
  # the report; a second one means `??`). Anything typed ahead of it, text on the line, or no
  # box leaves `?` as it was: the key's previous widget, or a plain `?`.
  typeset -gA _iohr_qm_prev
  _iohr_chat_qm() {
    if [[ -z $BUFFER && ${INORBIT_ASK-} != 0 && -n ${IOHR_PANEL_CHAT_FILE-} &&
          -n ${IOHR_PANEL-} && $PENDING == 0 ]] && kill -0 $IOHR_PANEL 2>/dev/null; then
      _iohr_chat_osc focus
      return 0
    fi
    zle ${_iohr_qm_prev[$KEYMAP]:-self-insert}
  }

  # The box's question, as the panel hands it over: the answer mode on the first line and the
  # `?` or `??` line on the second, in the private file; one Enter, no second one. A line on
  # the prompt is kept and comes back after the answer.
  _iohr_chat_send() {
    [[ ${INORBIT_ASK-} != 0 && -n ${IOHR_PANEL_CHAT_FILE-} && -r $IOHR_PANEL_CHAT_FILE ]] || return 0
    local mode line
    { IFS= read -r mode; IFS= read -r line } < $IOHR_PANEL_CHAT_FILE
    : >| $IOHR_PANEL_CHAT_FILE
    [[ $line == \?* ]] || return 0
    (( ${_iohr_ask_ring[(Ie)$mode]} )) && _iohr_ask_mode=$mode
    [[ -n $BUFFER ]] && zle push-line
    BUFFER=$line
    CURSOR=${#BUFFER}
    zle accept-line
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
    # Output attached from the menu goes with this ask only, and only for the command it was
    # attached to; the private file is removed once it is read.
    local outfile=$_iohr_ask_outfile outkind=$_iohr_ask_outkind
    _iohr_ask_outfile= _iohr_ask_outkind=
    if [[ -n $outfile && ${last[1,1024]} == $_iohr_ask_failcmd && -n $outkind ]]; then
      a+=(--output-file "$outfile" --output-kind "$outkind")
    fi
    _iohr_ask_failcmd= _iohr_ask_failrc=
    [[ -n $last ]] && { _iohr_ask_failcmd=${last[1,1024]}; _iohr_ask_failrc=$rc; }
    (( follow )) && [[ -n $_iohr_ask_pq ]] && a+=(--prior-question "$_iohr_ask_pq" --prior-answer "$_iohr_ask_pa")
    local rec rs=$'\x1e' rest tail
    # The card goes to the terminal on stderr. The record on stdout is the answer (kept for
    # `??`), the command it suggests, the link it names and the permission rules this run saved
    # (what Undo takes back), separated by a 0x1e character, then the folder Io needs a yes for and
    # the next steps.
    rec=$("${a[@]}" -- "$q"); local ok=$?
    [[ -n $outfile ]] && command rm -f -- "$outfile"
    if (( ok == 0 )); then
      rest=${rec#*$rs}
      _iohr_ask_pq=$q
      _iohr_ask_pa=${rec%%$rs*}
      _iohr_ask_cmd=${rest%%$rs*}
      tail=${rest#*$rs}
      _iohr_ask_link=${tail%%$rs*}
      # The later fields (undo, the folder Io needs a yes for, the next steps) may be missing
      # from an older record: each is then empty.
      local t2= t3=
      [[ $tail == *$rs* ]] && t2=${tail#*$rs}
      _iohr_ask_undo=${t2%%$rs*}
      [[ $t2 == *$rs* ]] && t3=${t2#*$rs}
      _iohr_ask_access=${t3%%$rs*}
      if [[ $t3 == *$rs* ]]; then _iohr_ask_steps=${t3#*$rs}; else _iohr_ask_steps=; fi
      _iohr_ask_live=1
    else
      _iohr_ask_pq= _iohr_ask_pa= _iohr_ask_cmd= _iohr_ask_link= _iohr_ask_undo= _iohr_ask_access= _iohr_ask_steps= _iohr_ask_live=
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
  # Last, so the box hears "ready" after the answer has been printed.
  add-zsh-hook precmd _iohr_chat_idle
  precmd_functions=(${precmd_functions:#_iohr_chat_idle} _iohr_chat_idle)
  add-zsh-hook preexec _iohr_chat_busy
  # Sourcing this again (a second `source ~/.zshrc`, an upgrade in a running shell, or a plugin
  # such as zsh-autosuggestions that re-wraps widgets) must not copy our own widget into
  # _iohr_ask_prev_accept, or accept-line would call itself until FUNCNEST. So the functions
  # above are redefined every time (a widget runs the function of its name), the accept-line
  # wiring is guarded by the widget that holds the previous accept-line (a shell variable can
  # be reset, the widget table is the truth), and the keys are bound again every time: only
  # the key's previous widget is kept from the first time, never one of ours.
  if (( ! ${+widgets[_iohr_ask_prev_accept]} )); then
    # Chain to whatever accept-line was before (the shell's own, or the user's widget).
    zle -A accept-line _iohr_ask_prev_accept
    zle -N accept-line _iohr_ask_accept
  fi
  # The menu key, in each keymap: the widget that held it is kept so the key keeps its
  # meaning anywhere else.
  if (( ${#_iohr_card_keys} )); then
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
  # The mode key (Shift+Tab, `^[[Z`), in each keymap, the same way.
  zle -N _iohr_ask_cycle
  local _iohr_km _iohr_cur
  for _iohr_km in emacs viins; do
    if [[ -z ${_iohr_mode_prev[$_iohr_km]-} ]]; then
      _iohr_cur=$(bindkey -M $_iohr_km -- '^[[Z' 2>/dev/null)
      _iohr_cur=${_iohr_cur##* }
      [[ -n $_iohr_cur && $_iohr_cur != undefined-key && $_iohr_cur != _iohr_ask_cycle ]] &&
        _iohr_mode_prev[$_iohr_km]=$_iohr_cur
    fi
    bindkey -M $_iohr_km -- '^[[Z' _iohr_ask_cycle
  done
  unset _iohr_km _iohr_cur
  # `?` (focus the box) and the private key the panel delivers a question with, in each keymap,
  # the same way. Both do nothing outside a panel with a box.
  zle -N _iohr_chat_qm
  zle -N _iohr_chat_send
  local _iohr_km _iohr_cur
  for _iohr_km in emacs viins; do
    if [[ -z ${_iohr_qm_prev[$_iohr_km]-} ]]; then
      _iohr_cur=$(bindkey -M $_iohr_km -- '?' 2>/dev/null)
      _iohr_cur=${_iohr_cur##* }
      [[ -n $_iohr_cur && $_iohr_cur != undefined-key && $_iohr_cur != _iohr_chat_qm ]] &&
        _iohr_qm_prev[$_iohr_km]=$_iohr_cur
    fi
    bindkey -M $_iohr_km -- '?' _iohr_chat_qm
    bindkey -M $_iohr_km -- '^[[27771~' _iohr_chat_send
  done
  unset _iohr_km _iohr_cur
fi
