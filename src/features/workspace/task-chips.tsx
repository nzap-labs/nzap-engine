import { useState } from 'react'
import { cn } from '@/lib/cn'

const TOPICS = [
  'Landing Page',
  'Knowledge/Teaching Material',
  '3D Modeling',
  'Mini Game',
  'Personal Blog',
]

/** Task-type chips under the composer; the selected one is ink-filled. */
export function TaskChips({ className }: { className?: string }) {
  const [selected, setSelected] = useState(0)
  return (
    <div
      role="group"
      aria-label="Task type"
      className={cn('flex flex-wrap justify-center gap-2', className)}
    >
      {TOPICS.map((topic, index) => (
        <button
          key={topic}
          type="button"
          aria-pressed={selected === index}
          onClick={() => setSelected(index)}
          className={cn(
            'cursor-pointer rounded-full border px-4 py-2 text-sm font-medium transition-colors',
            selected === index
              ? 'border-ink bg-ink text-paper'
              : 'border-ink bg-paper text-ink hover:bg-paper-soft',
          )}
        >
          {topic}
        </button>
      ))}
    </div>
  )
}
