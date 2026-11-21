# Computer use: Cryzo vs the ChatGPT app (2026-11-21)

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
