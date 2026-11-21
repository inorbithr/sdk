(function () {
  "use strict";
  // Drop the query (the one-time code and state) from the address bar and history.
  try { history.replaceState(null, "", location.pathname); } catch (e) {}

  var count = document.getElementById("count");
  if (count) {
    var n = 10;
    var num = document.getElementById("n");
    var stay = document.getElementById("stay");
    var say = function (text) { count.textContent = text; };
    count.hidden = false;
    var timer = setInterval(function () {
      n -= 1;
      if (n > 0) { num.textContent = String(n); return; }
      clearInterval(timer);
      // Only a tab a script opened can close itself; elsewhere this does nothing.
      try { window.close(); } catch (e) {}
      setTimeout(function () { say("Your browser kept this tab open. Close it whenever you like."); }, 250);
    }, 1000);
    stay.addEventListener("click", function () {
      clearInterval(timer);
      say("This tab will stay open.");
    });
  }

  var status = document.getElementById("status");
  var buttons = document.querySelectorAll("button[data-copy]");
  for (var i = 0; i < buttons.length; i++) {
    buttons[i].addEventListener("click", function (ev) {
      var btn = ev.currentTarget;
      var text = btn.getAttribute("data-copy");
      var hint = btn.querySelector(".hint");
      var done = function (ok) {
        if (hint) { hint.textContent = ok ? "copied" : "copy failed"; setTimeout(function () { hint.textContent = "copy"; }, 1600); }
        if (status) { status.textContent = ok ? "Copied " + text : "Could not copy " + text; }
      };
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(text).then(function () { done(true); }, function () { done(false); });
      } else { done(false); }
    });
  }
})();
