import { createFileRoute } from '@tanstack/react-router'
import { AppPage } from '@/features/apps/app-page'

export const Route = createFileRoute('/apps/$appId')({
  component: AppRoute,
})

function AppRoute() {
  const { appId } = Route.useParams()
  return <AppPage appId={appId} />
}
