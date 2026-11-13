/* eslint-disable */
/**
 * Generated `api` utility.
 *
 * THIS CODE IS AUTOMATICALLY GENERATED.
 *
 * To regenerate, run `npx convex dev`.
 * @module
 */

import type * as auth from "../auth.js";
import type * as diagnose from "../diagnose.js";
import type * as gateway from "../gateway.js";
import type * as http from "../http.js";
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
import type * as market_http from "../market/http.js";
import type * as market_store from "../market/store.js";
import type * as market_worker from "../market/worker.js";
import type * as quota from "../quota.js";

import type {
  ApiFromModules,
  FilterApi,
  FunctionReference,
} from "convex/server";

declare const fullApi: ApiFromModules<{
  auth: typeof auth;
  diagnose: typeof diagnose;
  gateway: typeof gateway;
  http: typeof http;
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
  "market/http": typeof market_http;
  "market/store": typeof market_store;
  "market/worker": typeof market_worker;
  quota: typeof quota;
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
