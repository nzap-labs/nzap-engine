import type { InputHTMLAttributes } from 'react'
import { cn } from '@/lib/cn'

export function Input({ className, ...props }: InputHTMLAttributes<HTMLInputElement>) {
  return (
    <input
      className={cn(
        'h-11 w-full rounded-lg border border-ink bg-paper px-3.5 text-[15px] text-ink outline-none',
        'placeholder:text-graphite focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink',
        'disabled:cursor-not-allowed disabled:bg-paper-soft disabled:text-graphite',
        className,
      )}
      {...props}
    />
  )
}
