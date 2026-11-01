# Cryzo Marketing review recordings

These recordings demonstrate real working features to reviewers. They are not advertisements. No recording has been supplied yet, and uploading a recording does not itself approve the app. Google basic sign-in is separate from the YouTube review below.

## Before recording

- Open the installed Cryzo app and Marketing. Use your own accounts and an original short test video you have permission to upload.
- In Windows Snipping Tool, choose Record, select the Cryzo and browser area, and start recording. Keep the consent screen and relevant account selection readable. Narrate what you click or add captions.
- Hide API keys, passwords, client secrets and unrelated personal information. Keep the app name, requested permissions and relevant consent details visible.
- Do not pretend a missing feature or failed operation succeeded. If a required control or connection is missing, stop and report that screen so it can be fixed before the final take.
- Publishing or uploading even a private test video is a real external action. If you want the agent to do it, approve the exact destination, video, caption/title and privacy setting separately first. You can perform your own test upload while recording.

## Recording 1: Google / YouTube

Purpose: show why Cryzo needs `youtube.upload`, `youtube.readonly` and `yt-analytics.readonly`, alongside basic identity.

1. Start with Cryzo open. State: “Cryzo Desktop lets a creator connect their own YouTube channel, upload a video they approve, and view that channel's performance.”
2. Open Marketing → Channels → Add channel → YouTube. Show the actual Google authorization flow, Cryzo Marketing YouTube identity, requested permissions, your chosen account/channel, consent and successful return to Cryzo. If an existing grant skips consent, arrange a fresh consent flow before the final recording rather than hiding this step.
3. Show the connected channel **Alu**, or whichever channel you genuinely authorized, and the available channel/content information. Explain this is the use of `youtube.readonly`.
4. In the real composer, choose that YouTube channel, an original short test video, an editable title/description and the privacy option actually offered. Show the preview and deliberate confirmation. Complete one upload you approve, then show its successful result. This demonstrates `youtube.upload`; opening a composer alone does not.
5. Open Marketing → Analytics for the same channel. Show the available report, date range and values. Explain this is the use of `yt-analytics.readonly`. Zero values are fine when they are the real response; unavailable metrics must remain described as unavailable.
6. Where supported, show disconnect or permission-denial behavior. Do not delete a real video just for the demonstration.
7. Upload the completed screen recording to YouTube as **Unlisted** and provide its watch URL. Check it can be viewed without your Google login. Do not make it Private, which prevents reviewers from opening it.

The review covers the Google project `cryzo-desktop`, not the separate `cryzo-sign-in` project. Include the consent and scope use for every OAuth client in the submitted project, if there is more than one. I can enter the real video URL, attach the prepared scope justification and submit the complete review once the evidence is ready.

Reference: [Google sensitive-scope verification](https://developers.google.com/identity/protocols/oauth2/production-readiness/sensitive-scope-verification).

## Recording 2: TikTok — prepare the sandbox first

**Do not record a final submission yet.** The production credentials exist, but the portal draft has not been saved because it requires an app icon and a genuine demo video. The sandbox connection and posting controls still need verification. A video cannot substitute for implementing missing controls.

For a first review, TikTok requires its developer-portal **Sandbox** environment. Sandbox credentials/test accounts must match the demonstrated integration; do not silently replace production credentials or present a production failure as a sandbox success.

1. Show the Cryzo Desktop sandbox app identity, then open the actual Cryzo Marketing flow.
2. Connect your TikTok test account through Login Kit. Show real consent, callback and the connected creator's identity (`user.info.basic`).
3. Show the creator statistics used by Cryzo (`user.info.stats`).
4. Select an original video and show the destination creator, editable caption, media preview and all required controls. Current creator information must determine the available privacy options and video duration limit. Select privacy yourself; it must not be preselected. Comment, Duet and Stitch choices must respect the creator's settings and must not start enabled. Show commercial-content disclosure and the relevant consent declaration.
5. Demonstrate **both** an upload-to-draft flow (`video.upload`) and a Direct Post flow (`video.publish`) if both scopes are retained. Show deliberate consent before transfer and the genuine processing/result status. If one flow is not implemented or not needed, remove that scope instead of claiming it works.
6. For an unaudited Direct Post client, follow the private-account/`SELF_ONLY` restrictions. Public Direct Post requires a separate audit. Do not attempt to bypass those restrictions.
7. Save the screen recording as MP4 or MOV, at most **50 MB per file**; the portal accepts up to five recordings. Provide the local file path so it can be uploaded to the review form.

I will finish the icon upload and saved configuration, check the real controls against the requirements, then submit a truthful complete application with the recording. Review approval and the Direct Post audit remain separate external decisions.

References: [TikTok app review](https://developers.tiktok.com/docs/en/app-review-guidelines), [TikTok content-sharing controls and audit](https://developers.tiktok.com/docs/en/content-sharing-guidelines).

## What else remains

- **LinkedIn:** the logo is uploaded and Cryzo Desktop app **264204164** is created. Share on LinkedIn and OpenID Connect are provisioned, the production callback is saved, and both credentials are saved/read-back verified on Convex production. Company Page association was approved and verified on Nov 1, 2026. No actual member OAuth connection is claimed yet. Organization posting still requires Community Management approval.
- **Meta / Facebook / Instagram:** the current session completed authorized phone verification but rejected the contact email during registration. You clarified that a developer account already exists, so duplicate registration is stopped. Open that existing account's **My Apps** dashboard in Chrome, or identify its Facebook login. App setup, permissions and any requested business verification/reviewer demonstration remain; this may require another recording and genuine business documents.
- **Reddit:** commercial inquiry **RR5KRM-0NGP3** is submitted; access approval remains pending.

You do not need to write the permission justifications or configure developer secrets yourself. The immediate contributions needed from you are Meta's account verification, genuine recordings once their flows are ready, and any platform-requested identity/business evidence. No social post is authorized by this guide.
