import { queryOptions, useMutation, useQueryClient } from '@tanstack/react-query'
import { call, stream } from '@/lib/ipc'
import type { Provider } from '@/types/colab'

/** One provider's connection, as Settings → Compute providers shows it. */
export interface ProviderStatus {
  provider: Exclude<Provider, 'colab'>
  connected: boolean
  /** The GitHub login, for Codespaces. */
  account: string | null
  detail: string | null
  error: string | null
}

export interface MachineOption {
  name: string
  label: string
  cpus: number | null
  memoryGb: number | null
}

export interface GithubUser {
  login: string
  name: string | null
  avatarUrl: string | null
}

export interface DeviceCode {
  userCode: string
  verificationUri: string
}

export const providersQuery = queryOptions({
  queryKey: ['providers', 'status'],
  queryFn: () => call<ProviderStatus[]>('providers_status'),
  staleTime: 30_000,
})

export function machinesQuery(provider: Exclude<Provider, 'colab'>, enabled = true) {
  return queryOptions({
    queryKey: ['providers', 'machines', provider],
    queryFn: () => call<MachineOption[]>('provider_machines', { provider }),
    staleTime: 5 * 60_000,
    enabled,
  })
}

function useInvalidateProviders() {
  const queryClient = useQueryClient()
  return () => void queryClient.invalidateQueries({ queryKey: ['providers'] })
}

export function useGithubToken() {
  const invalidate = useInvalidateProviders()
  return useMutation({
    mutationFn: (token: string) => call<GithubUser>('github_set_token', { token }),
    onSuccess: invalidate,
  })
}

export function useGithubCli() {
  const invalidate = useInvalidateProviders()
  return useMutation({
    mutationFn: () => call<GithubUser>('github_use_cli'),
    onSuccess: invalidate,
  })
}

/** GitHub's device flow: `onCode` receives the code to enter on github.com. */
export function useGithubDeviceLogin() {
  const invalidate = useInvalidateProviders()
  return useMutation({
    mutationFn: ({
      onCode,
      signal,
    }: {
      onCode: (code: DeviceCode) => void
      signal?: AbortSignal
    }) => stream<DeviceCode, GithubUser>('github_device_login', {}, { onEvent: onCode, signal }),
    onSuccess: invalidate,
  })
}

export function useGithubDisconnect() {
  const invalidate = useInvalidateProviders()
  return useMutation({ mutationFn: () => call('github_disconnect'), onSuccess: invalidate })
}

export function useCodesandboxKey() {
  const invalidate = useInvalidateProviders()
  return useMutation({
    mutationFn: (key: string) => call('codesandbox_set_key', { key }),
    onSuccess: invalidate,
  })
}

export function useCodesandboxDisconnect() {
  const invalidate = useInvalidateProviders()
  return useMutation({ mutationFn: () => call('codesandbox_disconnect'), onSuccess: invalidate })
}
