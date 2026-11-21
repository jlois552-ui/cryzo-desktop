import { Schema } from "effect"
import { Rpc, RpcGroup } from "effect/rpc"

const ServerReadyData = Schema.Struct({
  url: Schema.String,
})

export const AppAwaitInitialization = Rpc.make("AppAwaitInitialization", { success: ServerReadyData })

export const AppReconnectService = Rpc.make("AppReconnectService", { success: ServerReadyData })

export const AppConsumeInitialDeepLinks = Rpc.make("AppConsumeInitialDeepLinks", {
  success: Schema.Array(Schema.String),
})

export const AppIsFirstLaunchOnboardingPending = Rpc.make("AppIsFirstLaunchOnboardingPending", {
  success: Schema.Boolean,
})

export const AppFinishFirstLaunchOnboarding = Rpc.make("AppFinishFirstLaunchOnboarding", {
  payload: { createDefaultProject: Schema.Boolean },
  success: Schema.NullOr(Schema.String),
})

export const AppCheckAppExists = Rpc.make("AppCheckAppExists", {
  payload: { appName: Schema.String },
  success: Schema.Boolean,
})

export const AppResolveAppPath = Rpc.make("AppResolveAppPath", {
  payload: { appName: Schema.String },
  success: Schema.NullOr(Schema.String),
})

export const AppSetBackgroundColor = Rpc.make("AppSetBackgroundColor", {
  payload: { color: Schema.String },
})

export const AppExportDebugLogs = Rpc.make("AppExportDebugLogs", { success: Schema.String })

export const AppSetForceFocus = Rpc.make("AppSetForceFocus", {
  payload: { enabled: Schema.Boolean },
})

export const AppRecordFatalRendererError = Rpc.make("AppRecordFatalRendererError", {
  payload: {
    error: Schema.Struct({
      error: Schema.String,
      url: Schema.String,
      version: Schema.optionalKey(Schema.String),
      platform: Schema.String,
      os: Schema.optionalKey(Schema.String),
    }),
  },
})

export const AppSetNativeTranslations = Rpc.make("AppSetNativeTranslations", {
  payload: { value: Schema.Unknown },
})

export const AppRelaunch = Rpc.make("AppRelaunch")

const ComputerUse = Schema.Struct({
  enabled: Schema.Boolean,
  approval: Schema.Literals(["always", "per_app", "never"]),
  typeIntoPasswords: Schema.Boolean,
  blockedApps: Schema.Array(Schema.String),
})

// Null where this host has no computer use.
export const AppGetComputerUse = Rpc.make("AppGetComputerUse", { success: Schema.NullOr(ComputerUse) })

export const AppSetComputerUse = Rpc.make("AppSetComputerUse", { payload: { settings: ComputerUse } })

const RemoteControl = Schema.Struct({
  enabled: Schema.Boolean,
  connected: Schema.Boolean,
  clients: Schema.Number,
  error: Schema.NullOr(Schema.String),
  awake: Schema.Boolean,
  startup: Schema.Boolean,
  background: Schema.Boolean,
})

// Null where this host has no remote control.
export const AppGetRemoteControl = Rpc.make("AppGetRemoteControl", { success: Schema.NullOr(RemoteControl) })

export const AppSetRemoteControl = Rpc.make("AppSetRemoteControl", {
  payload: { enabled: Schema.Boolean },
  success: RemoteControl,
})

export const AppSetRemoteOptions = Rpc.make("AppSetRemoteOptions", {
  payload: {
    awake: Schema.optional(Schema.Boolean),
    startup: Schema.optional(Schema.Boolean),
    background: Schema.optional(Schema.Boolean),
  },
  success: RemoteControl,
})

export const AppPairRemoteDevice = Rpc.make("AppPairRemoteDevice", {
  success: Schema.Struct({ code: Schema.String, expiresAt: Schema.Number, url: Schema.NullOr(Schema.String) }),
})

export const AppListRemoteDevices = Rpc.make("AppListRemoteDevices", {
  success: Schema.Array(
    Schema.Struct({
      id: Schema.String,
      name: Schema.String,
      createdAt: Schema.Number,
      lastSeenAt: Schema.NullOr(Schema.Number),
    }),
  ),
})

export const AppRevokeRemoteDevice = Rpc.make("AppRevokeRemoteDevice", { payload: { id: Schema.String } })

export const AppRpcs = RpcGroup.make(
  AppAwaitInitialization,
  AppReconnectService,
  AppConsumeInitialDeepLinks,
  AppIsFirstLaunchOnboardingPending,
  AppFinishFirstLaunchOnboarding,
  AppCheckAppExists,
  AppResolveAppPath,
  AppSetBackgroundColor,
  AppExportDebugLogs,
  AppSetForceFocus,
  AppRecordFatalRendererError,
  AppSetNativeTranslations,
  AppRelaunch,
  AppGetComputerUse,
  AppSetComputerUse,
  AppGetRemoteControl,
  AppSetRemoteControl,
  AppSetRemoteOptions,
  AppPairRemoteDevice,
  AppListRemoteDevices,
  AppRevokeRemoteDevice,
)
