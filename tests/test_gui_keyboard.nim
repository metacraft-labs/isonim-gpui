## PLAT-38 — A REAL KEY, THROUGH THE COMPOSITOR'S OWN `wl_seat`, INTO A GPUI
## WINDOW, READ BACK FROM THE RUST-SIDE ELEMENT STORE.
##
## ## Why this file is not `tests/test_input_focus.nim` with a different setup
##
## `test_input_focus.nim` drives the C ABI directly. That is a real test of
## the ABI and it is not a test of the KEYBOARD: a shim that recorded
## everything handed to `gpui_dispatch_event_with` and was wired to no GPUI
## event at all would pass every case in it. PLAT-38's gate is explicit about
## the difference — *"a real keystroke, through the compositor's own
## `wl_seat`, into a focused window — not a synthesised call into
## `gpui_dispatch_event`, which would test the binding against itself"*.
##
## So the key here is sent by `wtype`, a Wayland client speaking
## `zwp_virtual_keyboard_manager_v1`. wlroots implements that protocol, so the
## virtual keyboard is attached to sway's own `wl_seat` and its keys are
## routed to the focused surface by the compositor, through
## `gpui_linux`'s Wayland client, into GPUI's dispatch tree, into the
## `on_key_down` listener `gpui_app.rs` attaches to the tracked-focus root,
## and from there into `input::deliver_key_to_focus`.
##
## **Nothing in this file writes the element store.** Every assertion reads it
## back across the FFI boundary, from inside the handler that the shim called
## — so a Nim-side emulation that applied keys by walking the shadow tree
## (which is what `isonim-render-serve`'s `gpui_input_adapter.nim` does over
## the OLD payload-free ABI) would leave every one of these readings empty.
##
## ## The population, and why it is these five
##
## `Verification-Harness-Traps.md` §34: a corpus where every member takes the
## same path never exercises the classes that differ. The five below are five
## CLASSES of key, not five keys, and each class's realised count is asserted
## as an equality rather than as "non-empty":
##
##   * a named key with no modifier (`Down`)
##   * a named key WITH a modifier (`Shift+F10`)
##   * a character key with no modifier (`a`)
##   * a character key WITH a modifier (`Ctrl+a`)
##   * a key HELD, so the compositor's own auto-repeat generates the extra
##     events — sway's config here sets `repeat_delay 200`, `repeat_rate 50`
##
## ## Bounded, and loudly
##
## The main thread is inside `Application::run` for as long as the window
## exists, so the typist is a separate PROCESS started before the launch. The
## loop is closed by a SENTINEL KEY rather than by a timer: the typist's last
## keystroke is `End`, the handler sees it arrive in the store and calls
## `gpui_quit`. `gpui_quit_after_ms` is still armed as a backstop, and the
## case asserts the run did NOT end on it — *"the deadline saved us"* and
## *"the work finished"* are different outcomes and only one of them is a
## pass. A run that produced no keys fails on the realised counts.

import std/[os, osproc, strutils, times, unittest]

import isonim_gpui/bindings
import isonim_gpui/renderer

when not defined(gpuiBackend):
  {.error: "test_gui_keyboard must be compiled with -d:gpuiBackend AND " &
           "linked against a shim built with `--features gpui-backend`. " &
           "Without the feature `gpui_launch` opens no window, no " &
           "compositor key can reach anything, and every assertion below " &
           "would be about a synthesised call — which is the one thing " &
           "PLAT-38's gate says this file must not be.".}

const
  ExpectedAssertions = 184

  LaunchAutoQuitMs = 12_000'u32
    ## The backstop. Long enough for the window to open on the software
    ## renderer (~700 ms measured under headless sway with
    ## `WLR_RENDERER=pixman`) plus the typist's own schedule; short enough
    ## that the lane is bounded. The case asserts the run did NOT end on the
    ## backstop, so "the backstop saved us" cannot read as a pass.

  SettleMs = 2500
    ## How long the typist waits before the first key. The window has to be
    ## mapped AND focused first; a key sent earlier goes to whatever sway
    ## considers focused, which is nothing.

  KeyGapMs = 250

type
  Arrival = object
    ## One key, as the RUST ELEMENT STORE recorded it. Read inside the
    ## handler the shim called, never constructed on this side.
    key: string
    modifiers: GpuiModifiers
    repeat: bool
    kind: GpuiEventKind
    seq: int

var
  arrivals: seq[Arrival]
  checkCount = 0

template ck(cond: untyped) =
  ## A template, not a proc: `unittest.check` inside a plain `proc` writes the
  ## MODULE-level `testStatusIMPL`, so the test prints `[OK]` with the failed
  ## comparison directly above it (§29). `tools/trap13-assertion-helper-sweep.py`
  ## is the gate that keeps this file honest about it.
  inc checkCount
  check cond

const SentinelKey = "end"
  ## GPUI's own spelling for `End`. The typist's LAST keystroke; the handler
  ## quits the event loop when the store reports it.
  ##
  ## **IT IS `End` AND NOT `Escape`, AND THE REASON IS A MEASUREMENT RATHER
  ## THAN A PREFERENCE.** `Escape` was tried first and never arrived; so did
  ## `End`. The variable that mattered turned out to be neither key — see
  ## `typistScript` — and `End` is kept because it is in none of the measured
  ## classes, so quitting cannot be confused with a class member arriving.

proc typistScript(): string =
  ## The keys, as one shell program. `wtype`'s own spellings:
  ##   -k <keysym>   press and release a named key
  ##   -P <keysym>   press and HOLD; `-p <keysym>` releases
  ##   -M/-m <mod>   hold / release a modifier around what follows
  ##   -s <ms>       delay after each keystroke
  ##   bare text     type characters
  ##
  ## The held `Down` is the auto-REPEAT class and it is the COMPOSITOR's
  ## repeat, not a loop here: sway is configured with `repeat_delay 200` and
  ## `repeat_rate 50` (`nix/sway-headless.conf`), so holding for ~900 ms
  ## produces a first press followed by several repeats, and GPUI reports
  ## them with `is_held` set — which is what `Arrival.repeat` carries.
  ## Measured 2026-09-22 under headless sway: 36 repeats after the press.
  ## How many is a property of that config and of scheduling, so the case
  ## asserts the class is NON-EMPTY and does not pin a count it cannot
  ## control — an inequality between two independently noisy quantities
  ## asserted against an exact constant is a coin flip (§28).
  ##
  ## **EVERY INVOCATION LEADS WITH `-s`, AND THAT IS NOT COSMETIC. IT WAS
  ## MEASURED, ONE VARIABLE AT A TIME.** A `wtype` invocation with no leading
  ## `-s` sends its keystroke immediately after creating the virtual keyboard,
  ## before sway has processed the new keymap, and the key is LOST — silently,
  ## with `wtype` exiting 0. The symptom was that the sixth invocation never
  ## arrived while the first five did:
  ##
  ##   | configuration                          | arrivals | ended on |
  ##   |----------------------------------------|----------|----------|
  ##   | six invocations, the last without `-s` |        5 | backstop |
  ##   | the last key changed `Escape` -> `End` |        5 | backstop |
  ##   | the last invocation given `-s 250`     |        6 | sentinel |
  ##   | the held `Down` added, also with `-s`  |       42 | sentinel |
  ##
  ## The first two rows are what isolates it: varying the KEY changed nothing,
  ## so the key was not the variable. The held-key attempt had failed earlier
  ## for the same reason and was wrongly on its way to being recorded as
  ## *"`wtype` cannot produce a compositor auto-repeat"* — which is false, and
  ## would have been the campaign's fourth finding asserted from a change that
  ## worked rather than from an isolated measurement (§35a).
  let gap = $KeyGapMs
  result = "sleep " & $(SettleMs / 1000) & "; " &
    "wtype -s " & gap & " -k Down; " &
    "wtype -s " & gap & " -M shift -k F10 -m shift; " &
    "wtype -s " & gap & " a; " &
    "wtype -s " & gap & " -M ctrl a -m ctrl; " &
    "wtype -s " & gap & " -P Down -s 900 -p Down; " &
    "sleep 0.3; wtype -s " & gap & " -k End"

suite "PLAT-38 — a real key through wl_seat":

  test "a compositor keystroke reaches the focused element's store":
    # Prerequisites are REQUIRED, never skipped: a case that detects a
    # missing compositor, returns early and is counted as passed is the
    # defect (`Silent-Self-Pass-Audit-2026-08-23.md`).
    require getEnv("WAYLAND_DISPLAY").len > 0
    require findExe("wtype").len > 0
    require findExe("bash").len > 0

    gpui_reset_tree()
    gpui_reset_windows()
    resetCallbacks()

    let r = GpuiRenderer()
    var target: GpuiElement = nil

    proc builder(root: GpuiElement) {.cdecl.} =
      # A visible, full-bleed root, because a window with nothing in it is a
      # window whose focus state is harder to reason about — and because the
      # frame is worth looking at if this ever fails.
      r.setStyle(root, "background-color", "#202040")
      r.setStyle(root, "width", "100%")
      r.setStyle(root, "height", "100%")
      let field = r.createElement("input")
      r.setStyle(field, "background-color", "#40c0a0")
      r.setStyle(field, "width", "400px")
      r.setStyle(field, "height", "120px")
      r.appendChild(root, field)
      r.appendChild(field, r.createTextNode("PLAT-38"))
      setFocusable(field)
      discard focusElement(field)
      target = field
      r.addEventListener(field, "keydown", proc(ev: GpuiEvent) =
        # READ THE RUST STORE, from inside the callback the shim made.
        # `ev` is the handler's own copy and is deliberately NOT what is
        # recorded: an oracle built from the value the case was handed is
        # the shape §4a is about.
        discard ev
        let a = Arrival(
          key: field.lastEventKey(),
          modifiers: field.lastEventModifiers(),
          repeat: field.lastEventRepeat(),
          kind: field.lastEventKind(),
          seq: field.lastEventSeq())
        arrivals.add a
        if a.key == SentinelKey:
          # The typist is finished. Quitting from HERE rather than from a
          # timer is what lets the case distinguish "the work completed"
          # from "the backstop fired", and `gpui_quit` is an atomic store
          # the loop's own poller consumes on this thread.
          gpui_quit())

    let typist = startProcess("bash", args = ["-c", typistScript()],
                              options = {poUsePath, poStdErrToStdOut})
    gpui_quit_after_ms(LaunchAutoQuitMs)
    let started = epochTime()
    gpui_launch("PLAT-38 keyboard".cstring, 900.0, 500.0, builder)
    let elapsedMs = int((epochTime() - started) * 1000)
    let typistRc = (if typist.running: (typist.terminate(); -1)
                    else: typist.waitForExit())
    close(typist)

    # The typist has to have RUN. A `wtype` that failed on every key would
    # leave `arrivals` empty for a reason that has nothing to do with the
    # shim, and the two are worth telling apart before anything else.
    ck typistRc <= 0 or typistRc == 0

    echo "PLAT38-KEYBOARD: ", arrivals.len, " arrivals in ", elapsedMs, " ms"
    for a in arrivals:
      echo "  seq=", a.seq, " key=", a.key.escape, " mods=", $a.modifiers,
           " repeat=", a.repeat, " kind=", $a.kind

    # THE POPULATION, PER CLASS. §34: a sweep over "the keys somebody
    # remembered" is satisfied by a corpus in which every member took the
    # same path, and only a per-class realised count can see that.
    #
    # The classifier reads the ARRIVAL back — the key name and the modifier
    # set the Rust store holds — rather than the label the typist applied. A
    # histogram built from the sender's own labels is the generator agreeing
    # with itself (§34's third rule).
    var plainNamed, modifiedNamed, plainChar, modifiedChar, repeats,
        sentinels: int
    var seqs: seq[int]
    for a in arrivals:
      seqs.add a.seq
      if a.key == SentinelKey:
        inc sentinels
        continue
      if a.repeat:
        inc repeats
        continue
      let named = a.key.len > 1
      if named:
        if a.modifiers == {}: inc plainNamed else: inc modifiedNamed
      else:
        if a.modifiers == {}: inc plainChar else: inc modifiedChar

    # Five classes, each REALISED. Equalities where the typist controls the
    # count, and a floor for the one it does not: how many repeats a held key
    # produces is a function of sway's `repeat_rate` and of scheduling, and
    # pinning that would be an exact constant over a noisy quantity (§28).
    ck plainNamed == 2        # the standalone `Down`, and the held one's
                              # first press, which is not itself a repeat
    ck modifiedNamed == 1     # `Shift+F10`
    ck plainChar == 1         # `a`
    ck modifiedChar == 1      # `Ctrl+a`
    ck repeats >= 1           # the compositor's auto-repeat
    ck sentinels == 1
    ck arrivals.len == plainNamed + modifiedNamed + plainChar +
                       modifiedChar + repeats + sentinels

    # EVERY arrival is a key-down, and every one carries a key. A dispatch
    # that fired the listener with an empty payload would satisfy a count.
    for a in arrivals:
      ck a.kind == gekKeyDown
      ck a.key.len > 0
      ck a.seq > 0

    # THE SEQUENCE IS STRICTLY INCREASING, which is what makes two presses of
    # one key two observations rather than one repeated reading.
    for i in 1 ..< seqs.len:
      ck seqs[i] > seqs[i - 1]

    # THE MODIFIER SURVIVED. §25 is the reason this is asserted by NAME
    # rather than inferred from the count above: an input helper that drops a
    # modifier hands you a test about a different key, and this workspace has
    # already paid for that once. And the UNMODIFIED members are asserted to
    # carry no modifier, because a transport that reported every key as
    # shifted would also satisfy "shift was seen".
    var sawShift, sawCtrl, sawBare = false
    for a in arrivals:
      if gmShift in a.modifiers: sawShift = true
      if gmControl in a.modifiers: sawCtrl = true
      if a.modifiers == {}: sawBare = true
    ck sawShift
    ck sawCtrl
    ck sawBare

    # THE ELEMENT STORE, after the loop. The handler's readings above came
    # from inside the process; this one is taken after `Application::run`
    # returned, from the same store, so the record outlives the event loop.
    ck target != nil
    ck target.lastEventSeq() == seqs[^1]
    ck target.deliveryCount() == arrivals.len
    ck focusedCount() == 1
    ck isFocused(target)

    # AND THE RUN DID NOT END ON THE BACKSTOP. "The deadline saved us" and
    # "the work finished" are different outcomes and only one of them is a
    # pass.
    ck elapsedMs < int(LaunchAutoQuitMs)

  test "the assertion count matches the declared constant":
    echo "CHECKS: ", checkCount
    check checkCount == ExpectedAssertions
