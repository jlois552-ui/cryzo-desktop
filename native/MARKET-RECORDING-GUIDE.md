# Cryzo Marketing review recordings

These recordings demonstrate real working features to reviewers. They are not advertisements. No recording has been supplied yet, and uploading a recording does not itself approve the app. Google basic sign-in is separate from the YouTube review below.

## Before recording

Prepared on November 2, 2026: Google Data Access is open in **Chrome**, with the scope explanation saved and its real YouTube-link field left empty. TikTok's review draft is also open in **Chrome**; it cannot save without an icon and genuine video. Meta stays in **Edge**; the approved Tech Provider designation unlocked Review → Verification and App Review. None of these reviews has been submitted.

| Recording | Current readiness | Where the evidence goes |
| --- | --- | --- |
| YouTube | Connected channel and analytics verified; record the actual consent and user-approved upload next. | [Google Data Access](https://console.cloud.google.com/auth/scopes?project=cryzo-desktop), YouTube link field. |
| TikTok | Not ready for a final take: Sandbox and posting controls need completion. | [Cryzo Desktop review draft](https://developers.tiktok.com/app/7702948353122928661/pending), App review → Upload. |
| Facebook / Instagram | Not ready for a final take: production secret, icon, real connections and business verification remain. | [Meta App Review](https://developers.facebook.com/apps/1120975114168572/app-review/submissions/) in Edge. |

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
4. In the real composer, choose that YouTube channel, an original short test video and an editable title/description. Deliberately choose **Private or Unlisted** for the test upload: the current composer defaults to Public. Show the preview and deliberate confirmation. Complete one upload you approve, then show its successful result. This demonstrates `youtube.upload`; opening a composer alone does not.
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

The code currently implements Direct Post but not the requested `video.upload` draft flow. Its fixed, preselected privacy options, missing Comment/Duet/Stitch and commercial-disclosure controls, and missing creator-duration enforcement need correction before a final recording. Do not film a mock interface or describe these controls as implemented. The portal draft is not persisted: Save still reports **App icon is required** and **Please upload at least one video**. Earlier product/scope selections did not survive reopening; they must be configured again before submission. Review approval and the Direct Post audit remain separate external decisions.

References: [TikTok app review](https://developers.tiktok.com/docs/en/app-review-guidelines), [TikTok content-sharing controls and audit](https://developers.tiktok.com/docs/en/content-sharing-guidelines).

## Recording 3: Meta — Facebook Page and Instagram

**Wait until the credential and connection blockers are resolved.** Test-ready permissions do not mean public access is approved. Use a Page and linked professional Instagram account you control, with appropriate app test roles until Advanced Access is approved.

1. Open Cryzo Desktop → Marketing → Channels. Show the Cryzo identity and genuine Facebook consent flow, requested permissions, account/Page selection and successful return. Keep passwords and tokens out of frame.
2. Show the administered Page list and chosen Page identity (`pages_show_list`). If business-owned asset selection is part of the reviewed flow, demonstrate its actual use of `business_management`; do not claim a nonexistent Business Manager screen demonstrates it.
3. Compose an original Page test post with your chosen text/media. Show the selected Page, preview and deliberate publish confirmation. Publish it yourself, or separately approve the exact post for the agent. Show the resulting Page post (`pages_manage_posts`).
4. Open that Page's available engagement and analytics, showing the real date range and response (`pages_read_engagement`, `read_insights`). Honest zero values are acceptable; failures must not be represented as success.
5. Connect the Page-linked Instagram Business/Creator account through the actual flow. Show the professional account identity (`instagram_basic`) and account selection.
6. Prepare original supported Instagram media and caption, show destination and preview, deliberately approve publication, then show the resulting Instagram post (`instagram_content_publish`). Show the same account/content insights (`instagram_manage_insights`).
7. Show disconnect, denial or reconnection where the review asks. Separate Facebook and Instagram recordings are fine if every requested permission and complete flow is clearly covered.
8. In Edge, open Review → App Review, attach the real recordings to the applicable permission requests, and provide exact reviewer steps and genuine test access through the portal's private fields. Never put real passwords or secrets in the video. Complete business verification with genuine documents when requested; a video does not replace it.

Do not submit until every requested permission has a truthful purpose and working demonstration. The approved Tech Provider classification is complete; business verification and Advanced Access remain separate pending decisions.

## What else remains

- **LinkedIn:** the logo is uploaded and Cryzo Desktop app **264204164** is created. Share on LinkedIn and OpenID Connect are provisioned, the production callback is saved, and both credentials are saved/read-back verified on Convex production. Company Page association was approved and verified on Nov 1, 2026. No actual member OAuth connection is claimed yet. Organization posting still requires Community Management approval.
- **Meta / Facebook / Instagram:** Edge reached the existing developer account and **Cryzo Desktop 1120975114168572** is created. Both production callbacks and eight adapter permissions are saved; permissions show Ready for testing, not public approval. Production app ID is set, but automatic approval review blocked saving the app secret even after specific consent; its temporary handoff was removed. The browser upload gate declined the icon upload. The user-approved Tech Provider classification unlocked the Review menu on November 2. Finish secret entry, icon, actual account/read verification and business verification before claiming review readiness. The older Composio-managed connection is separate and was not copied or migrated into the desktop.
- **Reddit:** commercial inquiry **RR5KRM-0NGP3** is submitted; access approval remains pending.

Permission justifications are prepared. Meta's secret save currently needs manual completion because automatic approval review blocked the CLI transfer; use the existing `set-market-secrets.ps1 -Network meta -Prod` hidden prompt from `services/model-gateway`. Do not paste secrets into recordings or source. Genuine recordings are needed once the flows work, along with any platform-requested identity/business evidence. No social post is authorized by this guide.
