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

/** Each category gets its own tint so the gallery scans at a glance. */
const CATEGORY_TINT: Record<AppCategory, string> = {
  audio: 'from-[#ffda6e] to-[#e78b72]',
  image: 'from-[#e78b72] to-[#b48cf2]',
  video: 'from-[#b48cf2] to-[#6ea8fe]',
  text: 'from-[#6ece9d] to-[#6ea8fe]',
  vision: 'from-[#6ea8fe] to-[#6ece9d]',
  data: 'from-[#c9c4b6] to-[#6f706b]',
  utility: 'from-[#d6d1c5] to-[#9a9b93]',
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
        'inline-grid size-12 shrink-0 place-items-center rounded-2xl bg-linear-to-br text-[#11110f] shadow-[inset_0_1px_0_rgba(255,255,255,0.45)]',
        CATEGORY_TINT[category] ?? CATEGORY_TINT.utility,
        className,
      )}
    >
      {createElement(appIcon(icon, category), { className: 'size-[45%]', strokeWidth: 2.2 })}
    </span>
  )
}
