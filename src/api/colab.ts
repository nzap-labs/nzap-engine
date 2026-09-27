import { queryOptions, useMutation, useQueryClient } from '@tanstack/react-query'
import { Channel } from '@tauri-apps/api/core'
import { call, stream, type StreamHandlers } from '@/lib/ipc'
import type {
  ColabAccount,
  ColabAssignment,
  ColabAutomationOp,
  ColabAutomationRequest,
  ColabConfig,
  ColabExecuteEvent,
  ColabFileListing,
  ColabFileModel,
  ColabHistoryEvent,
  ColabHistoryFormat,
  ColabImportedFile,
  ColabJobEvent,
  ColabJobRequest,
  ColabQuota,
  ColabResources,
  ColabRunFileEvent,
  ColabRunFileRequest,
  ColabSession,
  ColabStatus,
  CreateSessionRequest,
  GoogleUser,
  StopOutcome,
} from '@/types/colab'

/**
 * Connection status. Refetched often enough that the green dot tracks a token
 * dying on Google's side, but cached long enough that moving around the app
 * does not hammer the liveness probe (it calls through to Colab).
 */
export const colabStatusQuery = queryOptions({
  queryKey: ['colab', 'status'],
  queryFn: () => call<ColabStatus>('auth_status'),
  staleTime: 30_000,
  refetchOnWindowFocus: true,
})

export const colabConfigQuery = queryOptions({
  queryKey: ['colab', 'config'],
  queryFn: () => call<ColabConfig>('config_get'),
  staleTime: 5 * 60_000,
})

export const colabAccountQuery = queryOptions({
  queryKey: ['colab', 'account'],
  queryFn: () => call<ColabAccount>('account_get'),
  staleTime: 60_000,
})

/** Polled every minute, the same cadence as the extension's ConsumptionPoller. */
export const colabQuotaQuery = queryOptions({
  queryKey: ['colab', 'quota'],
  queryFn: () => call<ColabQuota>('quota_get'),
  staleTime: 30_000,
  refetchInterval: 60_000,
  retry: 1,
})

export const colabSessionsQuery = queryOptions({
  queryKey: ['colab', 'sessions'],
  queryFn: async () => ({ sessions: await call<ColabSession[]>('sessions_list') }),
  staleTime: 10_000,
  refetchInterval: 15_000,
})

export const colabAssignmentsQuery = queryOptions({
  queryKey: ['colab', 'assignments'],
  queryFn: async () => ({ assignments: await call<ColabAssignment[]>('assignments_list') }),
  staleTime: 30_000,
})

export const colabResourcesQuery = (name: string | null) =>
  queryOptions({
    queryKey: ['colab', 'resources', name],
    queryFn: async () => ({
      resources: await call<ColabResources>('session_resources', { name }),
    }),
    enabled: Boolean(name),
    staleTime: 5_000,
    refetchInterval: name ? 5_000 : false,
  })

export const colabFilesQuery = (name: string | null, path: string) =>
  queryOptions({
    queryKey: ['colab', 'files', name, path],
    queryFn: () => call<ColabFileListing>('files_list', { name, path }),
    enabled: Boolean(name),
    staleTime: 5_000,
  })

export const colabFileContentQuery = (name: string | null, path: string | null) =>
  queryOptions({
    queryKey: ['colab', 'file', name, path],
    queryFn: async () => ({ file: await call<ColabFileModel>('files_read', { name, path }) }),
    enabled: Boolean(name && path),
    staleTime: 10_000,
  })

export const colabHistoryQuery = (name: string | null) =>
  queryOptions({
    queryKey: ['colab', 'history', name],
    queryFn: async () => ({
      events: await call<ColabHistoryEvent[]>('history_get', { name, limit: 200 }),
    }),
    enabled: Boolean(name),
    staleTime: 5_000,
  })

/** `colab log -o <file>`: save the history in one of four formats. */
export function useExportHistory() {
  return useMutation({
    mutationFn: ({ name, format }: { name: string; format: ColabHistoryFormat }) =>
      call<string | null>('history_export', { name, format }),
  })
}

export function useClearHistory() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (name: string) => call<void>('history_clear', { name }),
    onSuccess: (_data, name) =>
      queryClient.invalidateQueries({ queryKey: ['colab', 'history', name] }),
  })
}

/** Invalidate everything Colab-related after a mutation. */
export function useInvalidateColab() {
  const queryClient = useQueryClient()
  return () => queryClient.invalidateQueries({ queryKey: ['colab'] })
}

/**
 * Connect Google: the engine opens the consent page in the browser and the
 * promise settles when the loopback redirect comes back (or fails).
 */
export function useConnectColab() {
  const invalidate = useInvalidateColab()
  return useMutation({
    mutationFn: (loginHint?: string) => call<GoogleUser>('auth_connect', { loginHint }),
    onSettled: () => invalidate(),
  })
}

export function cancelConnect(): Promise<void> {
  return call('auth_cancel')
}

/** The copy/paste fallback: open Google's page, then paste the code it shows. */
export function useRemoteConnect() {
  const invalidate = useInvalidateColab()
  return {
    begin: useMutation({
      mutationFn: (loginHint?: string) => call<string>('auth_begin_remote', { loginHint }),
    }),
    complete: useMutation({
      mutationFn: (code: string) => call<GoogleUser>('auth_complete_remote', { code }),
      onSuccess: () => invalidate(),
    }),
  }
}

export function useDisconnectColab() {
  const invalidate = useInvalidateColab()
  return useMutation({
    mutationFn: () => call<void>('auth_disconnect'),
    onSuccess: () => invalidate(),
  })
}

export function useCreateSession() {
  const invalidate = useInvalidateColab()
  return useMutation({
    mutationFn: (request: CreateSessionRequest) =>
      call<{ session: ColabSession; connected: boolean }>('session_create', {
        request: { name: request.name ?? '', ...request },
      }),
    onSuccess: () => invalidate(),
  })
}

type SessionAction =
  'connect' | 'disconnect' | 'keepalive' | 'restart' | 'interrupt' | 'drive/authorize'

const ACTION_COMMANDS: Record<SessionAction, string> = {
  connect: 'session_connect',
  disconnect: 'session_disconnect',
  keepalive: 'session_keepalive',
  restart: 'session_restart',
  interrupt: 'session_interrupt',
  'drive/authorize': 'session_drive_authorize',
}

export function useSessionAction() {
  const invalidate = useInvalidateColab()
  return useMutation({
    mutationFn: ({ name, action }: { name: string; action: SessionAction }) =>
      call<unknown>(ACTION_COMMANDS[action], { name }),
    onSuccess: () => invalidate(),
  })
}

export function useDeleteSession() {
  const invalidate = useInvalidateColab()
  return useMutation({
    mutationFn: (name: string) => call<StopOutcome>('session_stop', { name }),
    onSuccess: () => invalidate(),
  })
}

export function useSendStdin() {
  return useMutation({
    mutationFn: ({ name, value }: { name: string; value: string }) =>
      call<void>('session_stdin', { name, value }),
  })
}

export function useSaveFile() {
  const invalidate = useInvalidateColab()
  return useMutation({
    mutationFn: ({ name, path, content }: { name: string; path: string; content: string }) =>
      call<unknown>('files_write', { name, path, content }),
    onSuccess: () => invalidate(),
  })
}

export function useFileCommand() {
  const invalidate = useInvalidateColab()
  return useMutation({
    mutationFn: ({
      name,
      command,
      path,
    }: {
      name: string
      command: 'mkdir' | 'delete'
      path: string
    }) => call<unknown>(command === 'mkdir' ? 'files_mkdir' : 'files_delete', { name, path }),
    onSuccess: () => invalidate(),
  })
}

export function useRenameFile() {
  const invalidate = useInvalidateColab()
  return useMutation({
    mutationFn: ({ name, path, newPath }: { name: string; path: string; newPath: string }) =>
      call<unknown>('files_rename', { name, path, newPath }),
    onSuccess: () => invalidate(),
  })
}

/** Upload a picked or dropped file: raw bytes, target in headers. */
export function useUploadFile() {
  const invalidate = useInvalidateColab()
  return useMutation({
    mutationFn: async ({ name, path, file }: { name: string; path: string; file: File }) => {
      const bytes = new Uint8Array(await file.arrayBuffer())
      return call<unknown>('files_upload_bytes', bytes, {
        headers: {
          'x-nzap-session': encodeURIComponent(name),
          'x-nzap-path': encodeURIComponent(path),
        },
      })
    },
    onSuccess: () => invalidate(),
  })
}

/** Download a runtime file; the engine asks where to save it. */
export function useDownloadFile() {
  return useMutation({
    mutationFn: ({ name, path }: { name: string; path: string }) =>
      call<string | null>('files_download', { name, path }),
  })
}

export function useAssignmentAction() {
  const invalidate = useInvalidateColab()
  return useMutation({
    mutationFn: async ({ action, endpoint }: { action: 'adopt' | 'release'; endpoint: string }) => {
      if (action === 'adopt') await call<ColabSession>('assignment_adopt', { endpoint })
      else await call<void>('assignment_release', { endpoint })
    },
    onSuccess: () => invalidate(),
  })
}

/** Run a cell and stream its outputs. */
export async function streamExecute(
  name: string,
  code: string,
  handlers: StreamHandlers<ColabExecuteEvent>,
): Promise<void> {
  await stream('session_execute', { name, code }, handlers)
}

/** `colab install` / `drivemount` / `auth` on a runtime, streamed. */
export async function streamAutomation(
  name: string,
  op: ColabAutomationOp,
  request: ColabAutomationRequest,
  handlers: StreamHandlers<ColabExecuteEvent>,
): Promise<void> {
  await stream('session_automation', { name, op, request }, handlers)
}

/** `colab exec -f`: run a .py / .ipynb on a runtime, streaming every cell. */
export async function streamRunFile(
  name: string,
  request: ColabRunFileRequest,
  handlers: StreamHandlers<ColabRunFileEvent>,
): Promise<void> {
  await stream('session_run_file', { name, request }, handlers)
}

/** VS Code's "Import notebook from URL" (Colab, Drive, GitHub, https links). */
export function useImportNotebook() {
  return useMutation({
    mutationFn: (url: string) => call<ColabImportedFile>('import_notebook_url', { url }),
  })
}

/** Save text (an executed notebook, a job log) where the user chooses. */
export function saveTextFile(filename: string, content: string): Promise<string | null> {
  return call<string | null>('save_text_file', { filename, content })
}

/** `colab run`: fresh VM → script → artifacts → release, streamed. */
export async function streamJob(
  request: ColabJobRequest,
  handlers: StreamHandlers<ColabJobEvent>,
): Promise<void> {
  await stream('job_run', { request }, handlers)
}

/** Show a saved artifact in the system file manager. */
export function revealPath(path: string): Promise<void> {
  return call('reveal_path', { path })
}

export interface TerminalConnection {
  send: (frame: object) => void
  close: () => void
}

/**
 * Open the runtime's terminal. Output arrives as the upstream `{data}`
 * frames; `onClose` fires once when the shell ends.
 */
export async function openTerminal(
  name: string,
  handlers: { onData: (data: string) => void; onClose: (reason: string | null) => void },
): Promise<TerminalConnection> {
  const channel = new Channel<{ type: 'frame' | 'closed'; data?: string; reason?: string | null }>()
  channel.onmessage = (message) => {
    if (message.type === 'closed') {
      handlers.onClose(message.reason ?? null)
      return
    }
    try {
      const frame = JSON.parse(message.data ?? '') as { data?: unknown }
      if (typeof frame.data === 'string') handlers.onData(frame.data)
    } catch {
      // Only {data} frames carry terminal output.
    }
  }
  const id = await call<number>('terminal_open', { name, onFrame: channel })
  return {
    send: (frame) => {
      void call('terminal_send', { id, frame: JSON.stringify(frame) }).catch(() => undefined)
    },
    close: () => {
      void call('terminal_close', { id }).catch(() => undefined)
    },
  }
}
