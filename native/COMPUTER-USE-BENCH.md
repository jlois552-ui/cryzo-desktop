# Desktop computer-use benchmarks

## Vercel Labs desktop-control face-off

Use the [agent-desktop v1.3.1 release](https://github.com/vercel-labs/agent-desktop/releases/tag/v1.3.1). Check the Windows archive against the release's `checksums.txt` before extracting it. Run the following from the repository root after putting the MinGW compiler on PATH on Windows, or use the installed MSVC toolchain.

```powershell
cargo build --manifest-path native/Cargo.toml --locked -p cryzo-desktop-control --bin cryzo-desktop-mcp
bun native/desktop-control/eval/faceoff.ts --agent-desktop C:/tools/agent-desktop.exe --runs 5
```

The scripted phase creates two temporary windows with identical button labels. The window's own click counter verifies delivery. The runner records observation, lookup, and action time separately, then checks whether the pointer or foreground window changed. Vercel's default and headed modes have separate rows. Each run writes the executable hashes, raw observations, and a Markdown report to the printed temporary directory.

### Ticket transfer workflow

The long-form scripted run reads one native window, copies the ticket ID and two lines of notes to a matching form, marks it reviewed, and saves a draft. An identical decoy form detects writes to the wrong window. The grader requires the exact text, the checked box, and exactly one saved record. Each backend takes the same nine tool calls through a persistent MCP connection. Use release binaries for this comparison.

```powershell
cargo build --manifest-path native/Cargo.toml --locked --release -p cryzo-desktop-control --bin cryzo-desktop-mcp
bun native/desktop-control/eval/faceoff.ts --workflow --cryzo-mcp native/target/release/cryzo-desktop-mcp.exe --agent-desktop C:/tools/agent-desktop.exe --runs 6
```

The workflow report includes completed and failed trials, timings from the source read through the save acknowledgment, and the count of decoy events. The fixture resets between trials and rotates backend order. The task is scripted so the measurements isolate the desktop-control path.

On November 24, 2026, Cryzo and Vercel's default mode each saved six correct drafts with no decoy changes. Median elapsed times were 3.576 s for Cryzo and 4.046 s for Vercel default. Vercel headed refused physical input on the Notes field in all six trials. Use `--target-foreground` to try the same workflow with the destination initially active.

To compare a Windows polling change against the original build, save the original executable before rebuilding. Pass its path to `--baseline-cryzo-mcp` alongside the workflow command above. This interleaves both Cryzo builds and both Vercel modes. For a Cryzo build with settle profiling, set `$env:CRYZO_DESKTOP_PROFILE_SETTLE = '1'` to log the wait, sleep, and scan times to stderr.

On November 25, 2026, the revised poll schedule reduced Cryzo's median post-action wait from 494 ms to 457 ms across 24 profiled actions per build. The six-trial interleaved workflow measured 3,406 ms for the original Cryzo build, 3,198 ms for the revised build, and 3,189.5 ms for Vercel default. Each saved six exact drafts with zero decoy events. Vercel headed lost element focus before delivering input in all six trials. A separate Cryzo fixture verified that 275 ms delayed validation and save confirmation both appeared in the corresponding action responses.

The follow-up isolated action timing with the same profiling in both Cryzo builds. Moving the Windows cursor animation off the accessibility action's critical path cut the median preview from 220 ms to under 1 ms per action across 24 actions per build. The four action calls together fell from 2,895.5 ms to 2,100 ms at the median, compared with 1,413.5 ms for Vercel default. The same six-trial workflow took 3,151 ms before the change, 2,362 ms after the change, and 3,556.5 ms for Vercel default. Each saved six exact drafts with zero decoy events. Vercel headed lost element focus in all six trials. The revised Cryzo build returned delayed validation and save confirmation at both 275 ms and 350 ms in separate fixture checks. Vercel default was timed with its standard action responses and no optional [post-action selector wait](https://agent-browser.dev/desktop/configuration#post-action-wait-flags).

Use `--outcome-waits` with the same workflow command to have the current Cryzo build check a specific outcome after each action. The baseline and Vercel still use their standard responses. An `act` call can supply `"waitFor": "Draft saved"` to match an exact accessible name in the original window. A complete snapshot line without the element ID or focus suffix can check a more specific state. Cryzo verifies that the outcome is new and unique. An existing or unobserved match produces the normal window update and a note. Omit `waitFor` when the caller needs the window to settle and has no specific outcome in mind.

The six-trial outcome rerun used the same nine calls and save checks. All lanes used a fixture with a visible Draft saved label. The four action calls took a median of 807 ms with Cryzo outcome waits, 2,093.5 ms with the previous Cryzo build, and 1,386.5 ms with Vercel default. Whole-workflow medians were 1,166.5 ms, 2,400.5 ms, and 2,932 ms respectively. Each saved six exact drafts with zero recorded decoy events. Vercel headed lost focus in all six trials. Separate tests caught the 275 ms and 350 ms delayed validation and save messages with both Cryzo wait modes and confirmed that a missing or already-visible outcome falls back without repeating the action.

For the model phase, use a dedicated interactive Windows profile with no personal windows or documents. Sign in with ChatGPT there, quit Cryzo so the benchmark owns the model connection, and run the same commands with the model bridge built too.

```powershell
cargo build --manifest-path native/Cargo.toml --locked -p cryzo-engine --example faceoff_model
bun native/desktop-control/eval/faceoff.ts --agent-desktop C:/tools/agent-desktop.exe `
  --runs 5 --isolated-windows --model-bridge native/target/debug/examples/faceoff_model.exe `
  --oauth-db "$env:APPDATA/me.cryzo.desktop/cryzo.sqlite"
```

Both model-driven lanes use GPT-6.1 Sol High on the signed-in account. The driver makes fresh turns with the same tool schema and authorizes actions only in the trial's windows. It measures Calculator, Notepad creation and editing, Explorer, and Settings. Vercel uses headed mode in this phase for physical text input. Browser and OCR tasks are outside the native score. Setup and reset time are excluded. An interrupted or blocked trial stays visible in the native report.

Compare accuracy before timing. Advance Vercel if it matches Cryzo's accuracy with zero wrong-window actions and either reduces median task time by at least 20% or reliably completes a task Cryzo misses. Repeat the deciding cases before choosing a backend. The harness uses each backend independently and preserves the production Cryzo permission settings.

## Cryzo vs the ChatGPT app (2026-11-21)

The same ten Windows tasks (`native/desktop-control/eval/tasks.ts`), each checked against the real
end state (files, what the local web server received, values read from Windows), not against what
the agent says. Both agents ran with GPT-6.1 Sol: Cryzo through its OpenAI connection, the ChatGPT
desktop app on its default (GPT-6.1 Sol High, Work mode, Full access).

## Cryzo

`bun native/desktop-control/eval/run.ts --model openai:gpt-6.1-sol`

| Task | First run | After fixes |
|---|---|---|
| calculator | pass, 32 s, 4 calls | |
| notepad-new | pass, 40 s, 6 calls | |
| notepad-edit | pass, 21 s, 4 calls | |
| explorer | **fail**: hung, timed out at 300 s | pass 2/2, 28–35 s, 0 errors |
| settings | pass, 20 s, 5 calls | |
| browser-table | pass, 9 s, 1 call | |
| browser-form | lost to a model provider connection error | pass 2/2, 26–40 s |
| browser-download | pass, 8 s, 2 calls | |
| canvas-ocr | pass, 6 s, 1 call | |
| cross-app | pass, 31 s, 1 tool error | pass 2/2, 25–30 s, 0 errors |

All ten tasks pass. Passed tasks average about 21 s, with no screenshots.

## ChatGPT desktop app

Driven by typing each task into the app; approval waits are left out of its times.

- **calculator:** answered 248,171 (the check passes) after about 63 s of work. It reported that
  computer use "couldn't access" Calculator's controls, so it never clicked the buttons, and
  something typed "Calculator" into its own message box.
- **notepad-new:** stopped at its second approval prompt. The run was not completed.
- The other eight were not run.

The app asks for approval each time it uses a new app ("Allow ChatGPT to use Notepad?"), even on
Full access. Cryzo's automation safety check refused to answer those prompts on the user's behalf,
so the rest of the ChatGPT side needs the user at the PC: either answer each prompt, or choose
"Always allow this app" for Calculator, Notepad, File Explorer, Settings and the browser first.
Then start the harness and send each prompt:

```
bun native/desktop-control/eval/external.ts --agent chatgpt
curl "http://127.0.0.1:4799/start?task=explorer"        # the prompt to paste into ChatGPT
curl -X POST "http://127.0.0.1:4799/check?task=explorer" -d '{"answer":"<its reply>","seconds":42}'
curl "http://127.0.0.1:4799/summary"
```

## What the comparison found and fixed

1. **A frozen app froze every tool.** In File Explorer, right after Ctrl+Shift+N, the model typed
   into the `edit "Name"` cell of another file instead of the new folder's rename box. That cell
   reports a writable value, so setting it tried to rename that file in the middle of the new
   folder's rename. Explorer stopped answering for about a minute, and since `windows` asks every
   window (the taskbar is Explorer too), every later call waited on it.
   - Typing into a list row's cell now needs that cell to be the one being edited. Otherwise the
     tool says so in under a second, names the field that is being edited (here
     `edit "New folder"`), and suggests F2.
   - A read-only value is refused the same way, instead of forcing focus onto it.
   - UI Automation now gives up on a provider after 2 s to connect and 8 s per call, rather than
     waiting indefinitely.
2. **`act` with only a role.** Models write `{"role": "document", "action": "type"}` for Notepad's
   text. That failed with "act needs id or name"; a role alone now picks the first element with it.

## Not measured yet

- The ChatGPT side beyond calculator, as above.
- Several runs per task on both sides; this was one run each, plus two re-runs of the fixed tasks.
- Claude and Cryzo's free models on the same tasks.
