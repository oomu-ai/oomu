"use client";

import { useEffect } from "react";

const OPEN_RECOMMENDED_MODEL_SETTINGS_EVENT =
  "oomu://open-recommended-model-settings";

export function requestRecommendedModelSettings() {
  window.dispatchEvent(new Event(OPEN_RECOMMENDED_MODEL_SETTINGS_EVENT));
}

export function useRecommendedModelSettingsRoute(onOpen: () => void) {
  useEffect(() => {
    const handleOpen = () => onOpen();
    window.addEventListener(OPEN_RECOMMENDED_MODEL_SETTINGS_EVENT, handleOpen);
    return () => {
      window.removeEventListener(OPEN_RECOMMENDED_MODEL_SETTINGS_EVENT, handleOpen);
    };
  }, [onOpen]);
}
