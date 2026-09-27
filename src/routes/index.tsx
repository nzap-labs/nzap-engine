import { createFileRoute, redirect } from '@tanstack/react-router'

/** The app opens on the Colab workspace. */
export const Route = createFileRoute('/')({
  beforeLoad: () => {
    throw redirect({ to: '/colab' })
  },
})
