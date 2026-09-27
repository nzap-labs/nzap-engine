/**
 * Shared rendering for runtime output: stream text, tracebacks, status lines,
 * and Jupyter mime bundles (images, sandboxed HTML, plain text).
 */

export interface OutputBlock {
  id: number
  kind: 'stream' | 'result' | 'error' | 'status' | 'input'
  streamName?: string
  text?: string
  html?: string
  image?: { mime: string; data: string }
}

export function Block({ block }: { block: OutputBlock }) {
  if (block.kind === 'stream') {
    return (
      <pre className="whitespace-pre-wrap break-words">
        {block.streamName === 'stderr' ? (
          <span className="text-coral">{block.text}</span>
        ) : (
          block.text
        )}
      </pre>
    )
  }
  if (block.kind === 'error') {
    return <pre className="whitespace-pre-wrap break-words text-coral">{block.text}</pre>
  }
  if (block.kind === 'status') {
    return <p className="py-0.5 text-graphite">{block.text}</p>
  }
  return (
    <div className="py-1">
      {block.image ? (
        <img
          src={`data:${block.image.mime};base64,${block.image.data}`}
          alt="runtime output"
          className="max-w-full rounded-lg"
        />
      ) : block.html ? (
        <iframe
          // HTML output comes from the user's own runtime, so it is rendered in
          // a sandboxed document: no scripts, no same-origin access to NZAP.
          sandbox=""
          srcDoc={block.html}
          title="runtime output"
          className="min-h-16 w-full rounded-lg border-0 bg-white"
        />
      ) : (
        <pre className="whitespace-pre-wrap break-words">{block.text}</pre>
      )}
    </div>
  )
}

/** Render a Jupyter mime bundle into text, an image, or HTML. */
export function renderMimeBundle(data?: Record<string, string | string[]>): {
  text?: string
  html?: string
  image?: { mime: string; data: string }
} {
  if (!data) return {}
  const text = (key: string): string | undefined => {
    const value = data[key]
    if (value === undefined) return undefined
    return Array.isArray(value) ? value.join('') : value
  }
  if (data['image/png']) return { image: { mime: 'image/png', data: asString(data['image/png']) } }
  if (data['image/jpeg']) {
    return { image: { mime: 'image/jpeg', data: asString(data['image/jpeg']) } }
  }
  if (data['image/svg+xml']) {
    return { html: text('image/svg+xml') }
  }
  const html = text('text/html')
  if (html) return { html }
  return { text: text('text/plain') ?? '' }
}

function asString(value: string | string[]): string {
  return Array.isArray(value) ? value.join('') : value
}
