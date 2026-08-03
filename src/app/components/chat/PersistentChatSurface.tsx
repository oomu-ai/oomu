"use client";

import type { ComponentProps } from "react";
import { ChatScreen } from "../ChatScreen";

type PersistentChatSurfaceProps = ComponentProps<typeof ChatScreen> & {
  visible: boolean;
};

export function PersistentChatSurface({ visible, ...props }: PersistentChatSurfaceProps) {
  return (
    <div className={visible ? "contents" : "hidden"}>
      <ChatScreen {...props} isVisible={visible} />
    </div>
  );
}
