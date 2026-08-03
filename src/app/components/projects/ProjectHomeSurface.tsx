import { useAppShell } from "@/components/AppShell";
import { ProjectsScreen } from "./ProjectsScreen";

type ProjectHomeSurfaceProps = {
  onOpenChat: (projectId: string) => void;
};

export function ProjectHomeSurface({ onOpenChat }: ProjectHomeSurfaceProps) {
  const { setActiveItem, setWorkflowProjectScope } = useAppShell();
  return (
    <ProjectsScreen
      onOpenChat={onOpenChat}
      onOpenWorkflows={(scope) => {
        setActiveItem("workflows");
        setWorkflowProjectScope(scope);
      }}
    />
  );
}
