import { createElement } from 'react'
import {
  AudioLines,
  Bot,
  Box,
  Brain,
  Camera,
  Code,
  Eye,
  FileText,
  ImageIcon,
  Languages,
  MessageSquare,
  MicVocal,
  Music,
  Palette,
  ScanEye,
  Smile,
  Sparkles,
  Table,
  Video,
  WandSparkles,
  type LucideIcon,
} from 'lucide-react'
import { cn } from '@/lib/cn'
import type { AppCategory } from '@/types/app'

/** The icons an `app.json` may name (Lucide, kebab-case). */
const ICONS: Record<string, LucideIcon> = {
  'audio-lines': AudioLines,
  bot: Bot,
  box: Box,
  brain: Brain,
  camera: Camera,
  code: Code,
  eye: Eye,
  'file-text': FileText,
  image: ImageIcon,
  languages: Languages,
  'message-square': MessageSquare,
  'mic-vocal': MicVocal,
  music: Music,
  palette: Palette,
  'scan-eye': ScanEye,
  smile: Smile,
  sparkles: Sparkles,
  table: Table,
  video: Video,
  'wand-sparkles': WandSparkles,
}

const CATEGORY_ICON: Record<AppCategory, LucideIcon> = {
  audio: AudioLines,
  image: ImageIcon,
  video: Video,
  text: FileText,
  vision: ScanEye,
  data: Table,
  utility: Box,
}

/**
 * Brushed-chrome tiles, each category with a faint tint of its own so the
 * gallery still scans at a glance.
 */
const CATEGORY_TINT: Record<AppCategory, string> = {
  audio: 'from-[#ffffff] via-[#d7d9e2] to-[#8f95a8]',
  image: 'from-[#ffffff] via-[#e2d7dc] to-[#a68f99]',
  video: 'from-[#ffffff] via-[#dcd7e4] to-[#968fab]',
  text: 'from-[#ffffff] via-[#d7e2dd] to-[#8fa89c]',
  vision: 'from-[#ffffff] via-[#d7dfe4] to-[#8fa1ab]',
  data: 'from-[#ffffff] via-[#dededa] to-[#9d9d96]',
  utility: 'from-[#ffffff] via-[#dcdcdc] to-[#9a9a9a]',
}

function appIcon(icon: string | undefined, category: AppCategory): LucideIcon {
  return (icon && ICONS[icon]) || CATEGORY_ICON[category] || Sparkles
}

export function AppIcon({
  icon,
  category,
  className,
}: {
  icon?: string
  category: AppCategory
  className?: string
}) {
  return (
    <span
      aria-hidden
      className={cn(
        'inline-grid size-12 shrink-0 place-items-center rounded-2xl bg-linear-to-br text-[#0c0c0e] shadow-[inset_0_1px_0_rgba(255,255,255,0.9),inset_0_-1px_0_rgba(0,0,0,0.25),0_1px_2px_rgba(0,0,0,0.25)]',
        CATEGORY_TINT[category] ?? CATEGORY_TINT.utility,
        className,
      )}
    >
      {createElement(appIcon(icon, category), { className: 'size-[45%]', strokeWidth: 2.2 })}
    </span>
  )
}
