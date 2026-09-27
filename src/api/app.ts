import { queryOptions, useMutation, useQueryClient } from '@tanstack/react-query'
import { call } from '@/lib/ipc'

export interface AppInfo {
  version: string
  os: string
  arch: string
}

export interface Settings {
  catalogUrl: string
  keepAlive: boolean
  keepAliveIntervalSeconds: number
  closeToTray: boolean
  artifactsDir: string | null
}

export interface SettingsView {
  settings: Settings
  oauthClientId: string
  customOauthClient: boolean
  defaultCatalogUrl: string
}

export type SettingsPatch = Partial<Omit<Settings, 'artifactsDir'>> & { artifactsDir?: string }

export const appInfoQuery = queryOptions({
  queryKey: ['app', 'info'],
  queryFn: () => call<AppInfo>('app_info'),
  staleTime: Infinity,
})

export const settingsQuery = queryOptions({
  queryKey: ['app', 'settings'],
  queryFn: () => call<SettingsView>('settings_get'),
  staleTime: 60_000,
})

export function useUpdateSettings() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (patch: SettingsPatch) => call<SettingsView>('settings_update', { patch }),
    onSuccess: (view) => {
      queryClient.setQueryData(settingsQuery.queryKey, view)
      void queryClient.invalidateQueries({ queryKey: ['notebooks'] })
      void queryClient.invalidateQueries({ queryKey: ['colab', 'config'] })
    },
  })
}

/** Bring your own OAuth client (`null` restores the built-in one). */
export function useSetOAuthClient() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (json: string | null) => call<SettingsView>('settings_set_oauth_client', { json }),
    onSuccess: (view) => {
      queryClient.setQueryData(settingsQuery.queryKey, view)
      void queryClient.invalidateQueries({ queryKey: ['colab', 'status'] })
    },
  })
}

export function openLogFolder(): Promise<void> {
  return call('open_log_dir')
}
