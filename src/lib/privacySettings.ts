export type PrivacySettingsState = {
  automatedWebGroundingEnabled: boolean;
  licenseAccepted: boolean;
  licenseState: "not_presented" | "presented" | "accepted";
  acceptedLicenseVersion?: string | null;
  acceptanceTimestampMs?: number | null;
  licenseVersion: string;
  licenseEffectiveDate: string;
  licenseText: string;
};
