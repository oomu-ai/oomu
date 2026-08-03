export function validPngThumbnail(thumbnail) {
  if (
    thumbnail.mediaType !== "image/png" ||
    !Number.isInteger(thumbnail.width) ||
    !Number.isInteger(thumbnail.height) ||
    thumbnail.width < 300 ||
    thumbnail.height < 200
  ) {
    return false;
  }
  try {
    const bytes = Buffer.from(thumbnail.bytesBase64, "base64");
    const signature = "89504e470d0a1a0a";
    return bytes.length >= 24
      && bytes.subarray(0, 8).toString("hex") === signature
      && bytes.readUInt32BE(16) === thumbnail.width
      && bytes.readUInt32BE(20) === thumbnail.height;
  } catch {
    return false;
  }
}
