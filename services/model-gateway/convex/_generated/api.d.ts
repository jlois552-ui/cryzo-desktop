/* eslint-disable */
/**
 * Generated `api` utility.
 *
 * THIS CODE IS AUTOMATICALLY GENERATED.
 *
 * To regenerate, run `npx convex dev`.
 * @module
 */

import type * as apps_cloudflare from "../apps/cloudflare.js";
import type * as apps_http from "../apps/http.js";
import type * as apps_store from "../apps/store.js";
import type * as auth from "../auth.js";
import type * as cloud_http from "../cloud/http.js";
import type * as cloud_store from "../cloud/store.js";
import type * as cloud_worker from "../cloud/worker.js";
import type * as diagnose from "../diagnose.js";
import type * as gateway from "../gateway.js";
import type * as http from "../http.js";
import type * as lib_apps from "../lib/apps.js";
import type * as lib_cloud from "../lib/cloud.js";
import type * as lib_crypto from "../lib/crypto.js";
import type * as lib_device from "../lib/device.js";
import type * as lib_limits from "../lib/limits.js";
import type * as lib_market from "../lib/market.js";
import type * as lib_models from "../lib/models.js";
import type * as lib_providers_index from "../lib/providers/index.js";
import type * as lib_providers_linkedin from "../lib/providers/linkedin.js";
import type * as lib_providers_meta from "../lib/providers/meta.js";
import type * as lib_providers_net from "../lib/providers/net.js";
import type * as lib_providers_reddit from "../lib/providers/reddit.js";
import type * as lib_providers_tiktok from "../lib/providers/tiktok.js";
import type * as lib_providers_x from "../lib/providers/x.js";
import type * as lib_providers_youtube from "../lib/providers/youtube.js";
import type * as lib_ticket from "../lib/ticket.js";
import type * as lib_voice from "../lib/voice.js";
import type * as market_http from "../market/http.js";
import type * as market_store from "../market/store.js";
import type * as market_worker from "../market/worker.js";
import type * as quota from "../quota.js";
import type * as remote_http from "../remote/http.js";
import type * as remote_store from "../remote/store.js";
import type * as voice_http from "../voice/http.js";
import type * as voice_store from "../voice/store.js";
import type * as voice_whisper from "../voice/whisper.js";

import type {
  ApiFromModules,
  FilterApi,
  FunctionReference,
} from "convex/server";

declare const fullApi: ApiFromModules<{
  "apps/cloudflare": typeof apps_cloudflare;
  "apps/http": typeof apps_http;
  "apps/store": typeof apps_store;
  auth: typeof auth;
  "cloud/http": typeof cloud_http;
  "cloud/store": typeof cloud_store;
  "cloud/worker": typeof cloud_worker;
  diagnose: typeof diagnose;
  gateway: typeof gateway;
  http: typeof http;
  "lib/apps": typeof lib_apps;
  "lib/cloud": typeof lib_cloud;
  "lib/crypto": typeof lib_crypto;
  "lib/device": typeof lib_device;
  "lib/limits": typeof lib_limits;
  "lib/market": typeof lib_market;
  "lib/models": typeof lib_models;
  "lib/providers/index": typeof lib_providers_index;
  "lib/providers/linkedin": typeof lib_providers_linkedin;
  "lib/providers/meta": typeof lib_providers_meta;
  "lib/providers/net": typeof lib_providers_net;
  "lib/providers/reddit": typeof lib_providers_reddit;
  "lib/providers/tiktok": typeof lib_providers_tiktok;
  "lib/providers/x": typeof lib_providers_x;
  "lib/providers/youtube": typeof lib_providers_youtube;
  "lib/ticket": typeof lib_ticket;
  "lib/voice": typeof lib_voice;
  "market/http": typeof market_http;
  "market/store": typeof market_store;
  "market/worker": typeof market_worker;
  quota: typeof quota;
  "remote/http": typeof remote_http;
  "remote/store": typeof remote_store;
  "voice/http": typeof voice_http;
  "voice/store": typeof voice_store;
  "voice/whisper": typeof voice_whisper;
}>;

/**
 * A utility for referencing Convex functions in your app's public API.
 *
 * Usage:
 * ```js
 * const myFunctionReference = api.myModule.myFunction;
 * ```
 */
export declare const api: FilterApi<
  typeof fullApi,
  FunctionReference<any, "public">
>;

/**
 * A utility for referencing Convex functions in your app's internal API.
 *
 * Usage:
 * ```js
 * const myFunctionReference = internal.myModule.myFunction;
 * ```
 */
export declare const internal: FilterApi<
  typeof fullApi,
  FunctionReference<any, "internal">
>;

export declare const components: {
  betterAuth: import("../betterAuth/_generated/component.js").ComponentApi<"betterAuth">;
};
