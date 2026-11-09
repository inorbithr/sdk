# iohr theme: the InOrbit prompt for zsh (RFC 0212). It is appended to `iohr shell hook zsh`
# only after `iohr theme install NAME --prompt`, and `iohr theme off` takes it away again.
# The colours are the theme's, the same list the InOrbit app's terminal and the Terminal.app
# profile are made from.
#
#   ~/development/core  main  ✘ 1      the folder, the git branch, the last status when it failed
#   ❯                                  the accent colour; red after a failed command
#
# The right prompt (the "3 asks" segment) belongs to the ask hook above and is not touched.
#
# Your own prompt wins. The prompt is replaced only when PROMPT is empty, is zsh's own default
# (`%m%# `, or macOS's `%n@%m %1~ %# `), or is the one this snippet set; a prompt you or a
# framework set stays, and nothing here says otherwise. `iohr theme install NAME --replace-prompt`
# is the way to say it. INORBIT_PROMPT=0 turns it off for one shell. INORBIT_PROMPT_DIRTY=1
# adds a mark when the working tree has changes (slower in a big repository).
if [[ -o interactive && -z ${functions[_iohr_prompt_precmd]-} && ${INORBIT_PROMPT-} != 0 ]]; then
  # A terminal without 24-bit colour (Terminal.app) gets the nearest of its 256 colours.
  [[ ${COLORTERM-} == (truecolor|24bit) ]] || zmodload zsh/nearcolor 2>/dev/null
  autoload -Uz vcs_info add-zsh-hook
  # What this snippet owns: its own vcs_info context, so a framework's vcs_info is not changed.
  zstyle ':vcs_info:*:iohr:*' enable git
  zstyle ':vcs_info:git:iohr:*' formats '%b'
  zstyle ':vcs_info:git:iohr:*' actionformats '%b|%a'
  zstyle ':vcs_info:git:iohr:*' stagedstr '*'
  zstyle ':vcs_info:git:iohr:*' unstagedstr '*'
  typeset -g _iohr_prompt_mode='__IOHR_PROMPT_MODE__'
  typeset -g _iohr_prompt_last=
  typeset -g _iohr_c_cwd='__IOHR_C_CWD__' _iohr_c_git='__IOHR_C_GIT__' _iohr_c_dirty='__IOHR_C_DIRTY__'
  typeset -g _iohr_c_ok='__IOHR_C_OK__' _iohr_c_bad='__IOHR_C_BAD__'
  if [[ ${LC_ALL:-${LC_CTYPE:-$LANG}} == *[Uu][Tt][Ff]-8* || ${LC_ALL:-${LC_CTYPE:-$LANG}} == *[Uu][Tt][Ff]8* ]]; then
    typeset -g _iohr_g_ok='❯' _iohr_g_bad='✘'
  else
    typeset -g _iohr_g_ok='>' _iohr_g_bad='x'
  fi

  _iohr_prompt_precmd() {
    # Not ours (any more): leave it, unless the person asked for it to be replaced.
    if [[ $_iohr_prompt_mode != replace && $PROMPT != $_iohr_prompt_last ]]; then
      case $PROMPT in
        ''|'%m%# '|'%n@%m %1~ %# '|'%n@%m %~ %# ') ;;
        *) return 0 ;;
      esac
    fi
    local git=''
    if [[ -n ${INORBIT_PROMPT_DIRTY-} && ${INORBIT_PROMPT_DIRTY-} != 0 ]]; then
      zstyle ':vcs_info:git:iohr:*' check-for-changes true
      zstyle ':vcs_info:git:iohr:*' formats '%b%c%u'
    else
      zstyle ':vcs_info:git:iohr:*' check-for-changes false
      zstyle ':vcs_info:git:iohr:*' formats '%b'
    fi
    vcs_info iohr 2>/dev/null
    if [[ -n ${vcs_info_msg_0_-} ]]; then
      local b=${vcs_info_msg_0_//\%/%%}
      if [[ $b == *'*' ]]; then
        git="  ${_iohr_c_git}${b%\*}%f${_iohr_c_dirty}*%f"
      else
        git="  ${_iohr_c_git}${b}%f"
      fi
    fi
    PROMPT="${_iohr_c_cwd}%~%f${git}%(?..  ${_iohr_c_bad}${_iohr_g_bad} %?%f)"$'\n'"%(?.${_iohr_c_ok}.${_iohr_c_bad})${_iohr_g_ok}%f "
    _iohr_prompt_last=$PROMPT
  }
  add-zsh-hook precmd _iohr_prompt_precmd
fi
