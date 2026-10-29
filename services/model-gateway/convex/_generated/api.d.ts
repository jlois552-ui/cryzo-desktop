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
import type * as lib_device from "../lib/device.js";
import type * as lib_limits from "../lib/limits.js";
import type * as lib_models from "../lib/models.js";
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
  "lib/device": typeof lib_device;
  "lib/limits": typeof lib_limits;
  "lib/models": typeof lib_models;
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
