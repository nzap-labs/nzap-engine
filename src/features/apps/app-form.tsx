import { useId, useRef, type ReactNode } from 'react'
import { ChevronDown, FileAudio, Upload, X } from 'lucide-react'
import { cn } from '@/lib/cn'
import type { NotebookParamValues } from '@/types/notebook'
import { fieldLabel, optionLabel, visibleOptions, type FormField, type FormSection } from './spec'

const control =
  'w-full rounded-2xl border border-ink bg-transparent text-sm outline-none placeholder:text-graphite/60 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink disabled:opacity-60'

const pill = (active: boolean) =>
  cn(
    'h-9 cursor-pointer rounded-3xl border px-4 text-sm font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-60',
    'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink',
    active ? 'border-ink bg-sunshine text-on-sunshine' : 'border-ink text-ink hover:bg-paper-soft',
  )

export interface AppFormProps {
  sections: FormSection[]
  values: NotebookParamValues
  files: Record<string, File>
  disabled?: boolean
  onChange: (key: string, value: string | number | boolean) => void
  onFile: (key: string, file: File | null) => void
}

/** The form an app spec describes, one widget per parameter. */
export function AppForm({ sections, ...props }: AppFormProps) {
  return (
    <div className="space-y-5">
      {sections.map((section) =>
        section.title === null ? (
          <Fields key="main" fields={section.fields} {...props} />
        ) : (
          <details
            key={section.title}
            open={!section.collapsed}
            className="group rounded-2xl border border-line px-4 py-3"
          >
            <summary className="flex cursor-pointer list-none items-center justify-between text-sm font-medium">
              {section.title}
              <ChevronDown className="size-4 text-graphite transition-transform group-open:rotate-180" />
            </summary>
            <div className="mt-4">
              <Fields fields={section.fields} {...props} />
            </div>
          </details>
        ),
      )}
    </div>
  )
}

function Fields({ fields, ...props }: Omit<AppFormProps, 'sections'> & { fields: FormField[] }) {
  return (
    <div className="space-y-5">
      {fields.map((field) => (
        <Field key={field.param.key} field={field} {...props} />
      ))}
    </div>
  )
}

function Labelled({
  id,
  field,
  extra,
  children,
}: {
  id: string
  field: FormField
  extra?: ReactNode
  children: ReactNode
}) {
  return (
    <div>
      <div className="flex items-baseline justify-between gap-3">
        <label
          htmlFor={id}
          className="text-xs font-medium uppercase tracking-[0.14em] text-graphite"
        >
          {fieldLabel(field)}
          {field.param.required && <span aria-hidden> *</span>}
        </label>
        {extra}
      </div>
      <div className="mt-1.5">{children}</div>
      {field.param.description && (
        <p className="mt-1.5 text-xs leading-relaxed text-graphite">{field.param.description}</p>
      )}
    </div>
  )
}

function Field({
  field,
  values,
  files,
  disabled,
  onChange,
  onFile,
}: Omit<AppFormProps, 'sections'> & { field: FormField }) {
  const id = useId()
  const { param, input } = field
  const value = values[param.key]
  const set = (next: string | number | boolean) => onChange(param.key, next)

  switch (input.widget) {
    case 'textarea': {
      const text = String(value ?? '')
      return (
        <Labelled
          id={id}
          field={field}
          extra={
            input.maxLength ? (
              <span className="text-xs tabular-nums text-graphite">
                {text.length.toLocaleString()} / {input.maxLength.toLocaleString()}
              </span>
            ) : null
          }
        >
          <textarea
            id={id}
            value={text}
            rows={input.rows ?? 4}
            maxLength={input.maxLength}
            placeholder={input.placeholder}
            disabled={disabled}
            onChange={(event) => set(event.target.value)}
            className={cn(control, 'resize-y px-4 py-3 leading-relaxed')}
          />
        </Labelled>
      )
    }
    case 'input':
      return (
        <Labelled id={id} field={field}>
          <input
            id={id}
            value={String(value ?? '')}
            maxLength={input.maxLength}
            placeholder={input.placeholder}
            disabled={disabled}
            onChange={(event) => set(event.target.value)}
            className={cn(control, 'h-11 px-4')}
          />
        </Labelled>
      )
    case 'file':
      return (
        <Labelled id={id} field={field}>
          <FilePicker
            id={id}
            accept={input.accept}
            maxMb={input.maxMb}
            file={files[param.key] ?? null}
            disabled={disabled}
            onFile={(file) => onFile(param.key, file)}
          />
        </Labelled>
      )
    case 'segmented':
    case 'radio': {
      const options = visibleOptions(field, values)
      return (
        <Labelled id={id} field={field}>
          <div
            id={id}
            role="radiogroup"
            aria-label={fieldLabel(field)}
            className="flex flex-wrap gap-2"
          >
            {options.map((option) => (
              <button
                key={option}
                type="button"
                role="radio"
                aria-checked={value === option}
                disabled={disabled}
                onClick={() => set(option)}
                className={pill(value === option)}
              >
                {optionLabel(field, option)}
              </button>
            ))}
          </div>
        </Labelled>
      )
    }
    case 'select': {
      const options = visibleOptions(field, values)
      return (
        <Labelled id={id} field={field}>
          <select
            id={id}
            value={String(value ?? '')}
            disabled={disabled}
            onChange={(event) => set(event.target.value)}
            className={cn(control, 'h-11 px-3')}
          >
            {options.map((option) => (
              <option key={option} value={option}>
                {optionLabel(field, option)}
              </option>
            ))}
          </select>
        </Labelled>
      )
    }
    case 'slider': {
      const number = Number(value ?? input.min ?? 0)
      const step = input.step ?? (param.type === 'integer' ? 1 : 0.1)
      const decimals = String(step).split('.')[1]?.length ?? 0
      return (
        <Labelled
          id={id}
          field={field}
          extra={
            <span className="text-sm font-medium tabular-nums">
              {number.toFixed(decimals)}
              {input.unit ?? ''}
            </span>
          }
        >
          <input
            id={id}
            type="range"
            min={input.min}
            max={input.max}
            step={step}
            value={number}
            disabled={disabled}
            onChange={(event) => set(Number(event.target.value))}
            className="h-2 w-full cursor-pointer accent-[var(--color-ink)]"
          />
        </Labelled>
      )
    }
    case 'number':
      return (
        <Labelled id={id} field={field}>
          <input
            id={id}
            type="number"
            inputMode={param.type === 'integer' ? 'numeric' : 'decimal'}
            min={input.min}
            max={input.max}
            step={input.step ?? (param.type === 'integer' ? 1 : 'any')}
            value={value === undefined ? '' : String(value)}
            disabled={disabled}
            onChange={(event) => set(event.target.value === '' ? '' : Number(event.target.value))}
            className={cn(control, 'h-11 px-4 tabular-nums')}
          />
        </Labelled>
      )
    case 'switch':
    case 'checkbox':
      return (
        <div className="flex items-start justify-between gap-4">
          <div>
            <label htmlFor={id} className="text-sm font-medium">
              {fieldLabel(field)}
            </label>
            {param.description && (
              <p className="mt-0.5 text-xs leading-relaxed text-graphite">{param.description}</p>
            )}
          </div>
          <button
            id={id}
            type="button"
            role="switch"
            aria-checked={Boolean(value)}
            disabled={disabled}
            onClick={() => set(!value)}
            className={cn(
              'relative h-6 w-11 shrink-0 cursor-pointer rounded-full border border-ink transition-colors disabled:opacity-60',
              'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink',
              value ? 'bg-sunshine' : 'bg-paper-soft',
            )}
          >
            <span
              aria-hidden
              className={cn(
                'absolute top-0.5 size-[18px] rounded-full bg-[#11110f] transition-transform',
                value ? 'translate-x-[22px]' : 'translate-x-0.5',
              )}
            />
          </button>
        </div>
      )
  }
}

function FilePicker({
  id,
  accept,
  maxMb,
  file,
  disabled,
  onFile,
}: {
  id: string
  accept?: string
  maxMb?: number
  file: File | null
  disabled?: boolean
  onFile: (file: File | null) => void
}) {
  const inputRef = useRef<HTMLInputElement>(null)
  const tooBig = Boolean(file && maxMb && file.size > maxMb * 1024 * 1024)

  function pick(picked: File | undefined) {
    if (picked) onFile(picked)
  }

  if (file) {
    return (
      <div
        className={cn(
          'flex items-center gap-3 rounded-2xl border px-4 py-3 text-sm',
          tooBig ? 'border-coral' : 'border-ink',
        )}
      >
        <FileAudio className="size-4 shrink-0 text-graphite" />
        <span className="min-w-0 flex-1 truncate">{file.name}</span>
        <span className="shrink-0 text-xs tabular-nums text-graphite">
          {(file.size / 1024 / 1024).toFixed(1)} MB
        </span>
        <button
          type="button"
          aria-label={`Remove ${file.name}`}
          disabled={disabled}
          onClick={() => onFile(null)}
          className="cursor-pointer rounded-lg p-1 text-graphite hover:bg-paper-soft hover:text-ink"
        >
          <X className="size-4" />
        </button>
        {tooBig && <span className="sr-only">File is larger than {maxMb} MB</span>}
      </div>
    )
  }

  return (
    <div
      onDragOver={(event) => event.preventDefault()}
      onDrop={(event) => {
        event.preventDefault()
        if (!disabled) pick(event.dataTransfer.files[0])
      }}
      className="flex items-center justify-between gap-3 rounded-2xl border border-dashed border-ink px-4 py-3 text-sm text-graphite"
    >
      <span>Drop a file here{maxMb ? ` (up to ${maxMb} MB)` : ''}</span>
      <button
        type="button"
        disabled={disabled}
        onClick={() => inputRef.current?.click()}
        className="inline-flex h-8 cursor-pointer items-center gap-1.5 rounded-3xl border border-ink px-3 text-xs font-medium text-ink hover:bg-paper-soft"
      >
        <Upload className="size-3.5" /> Choose
      </button>
      <input
        ref={inputRef}
        id={id}
        type="file"
        accept={accept}
        className="sr-only"
        tabIndex={-1}
        onChange={(event) => {
          pick(event.target.files?.[0])
          event.target.value = ''
        }}
      />
    </div>
  )
}

/** Files that exceed their widget's size limit, as messages. */
export function fileProblems(sections: FormSection[], files: Record<string, File>): string[] {
  const problems: string[] = []
  for (const section of sections)
    for (const field of section.fields) {
      const file = files[field.param.key]
      const maxMb = field.input.maxMb
      if (file && maxMb && file.size > maxMb * 1024 * 1024)
        problems.push(`${fieldLabel(field)} must be at most ${maxMb} MB.`)
    }
  return problems
}
