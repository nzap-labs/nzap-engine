import { createFileRoute } from '@tanstack/react-router'
import { ColabWorkspace } from '@/features/colab/colab-workspace'

export const Route = createFileRoute('/colab')({
  component: ColabWorkspace,
})
