export const isDeveloperBuild =
  process.env.NODE_ENV !== "production" ||
  process.env.NEXT_PUBLIC_OOMU_DEVELOPER_BUILD === "1" ||
  process.env.NEXT_PUBLIC_OOMU_DEVELOPER_BUILD === "true";
