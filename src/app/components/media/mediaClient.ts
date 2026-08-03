import { invoke } from "@/lib/invoke";

type TranscriptRecord={revision:number;transcript:string;language:string;confidence:number|null;timestamps:Array<{startMs:number;endMs:number;text:string}>;routeKind:"local"|"provider"|"manual";routeLabel:string;editedByUser:boolean;createdAtMs:number};
export type MediaAsset={mediaAssetId:string;projectId:string;taskId:string|null;taskRunId:string|null;mediaKind:"audio"|"image"|"video";mimeType:string;sha256:string;byteLength:number;sourceKind:string;sourceReference:string;width:number|null;height:number|null;durationMs:number|null;retentionMode:string;expiresAtMs:number|null;redactionState:string;redactionCategories:string[];routingMode:string;providerIds:string[];createdAtMs:number;latestTranscript:TranscriptRecord|null;relatedAssetIds:string[]};
type MediaData={mediaAssetId:string;mimeType:string;dataBase64:string;sha256:string};
type MediaInterpretation={revision:number;interpretationKind:"local_vision"|"alt_text";text:string;routeLabel:string;editedByUser:boolean;createdAtMs:number};

export const mediaApi={
  list:(projectId:string)=>invoke<MediaAsset[]>("list_media_assets",{request:{projectId}}),
  ingest:(request:Record<string,unknown>)=>invoke<MediaAsset>("ingest_media_asset",{request}),
  data:(projectId:string,mediaAssetId:string)=>invoke<MediaData>("get_media_asset_data",{request:{projectId,mediaAssetId}}),
  transcript:(request:Record<string,unknown>)=>invoke<TranscriptRecord>("save_media_transcript",{request}),
  remove:(projectId:string,mediaAssetId:string)=>invoke("delete_media_asset",{request:{projectId,mediaAssetId}}),
  sanitize:(projectId:string,mediaAssetId:string)=>invoke<MediaAsset>("sanitize_media_image",{request:{projectId,mediaAssetId}}),
  analyze:(projectId:string,mediaAssetId:string)=>invoke<MediaInterpretation>("analyze_media_image",{request:{projectId,mediaAssetId}}),
  altText:(projectId:string,mediaAssetId:string,text:string)=>invoke<MediaInterpretation>("save_media_alt_text",{request:{projectId,mediaAssetId,text}}),
};
