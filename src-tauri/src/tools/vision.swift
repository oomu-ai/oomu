import Foundation
import Vision
import AppKit

struct TextItem: Codable {
    let text: String
    let confidence: Float
    let page: Int?
}

struct ClassItem: Codable {
    let label: String
    let confidence: Float
}

struct Output: Codable {
    var backend: String = "apple-vision-local"
    var width: Int?
    var height: Int?
    var page_count: Int?
    var texts: [TextItem] = []
    var classifications: [ClassItem] = []
    var warnings: [String] = []
}

func cgImage(from image: NSImage) -> CGImage? {
    var rect = NSRect(origin: .zero, size: image.size)
    return image.cgImage(forProposedRect: &rect, context: nil, hints: nil)
}

func recognizeText(in cgImage: CGImage, page: Int?, output: inout Output) {
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.usesLanguageCorrection = true
    if #available(macOS 13.0, *) {
        request.revision = VNRecognizeTextRequestRevision3
    }
    let handler = VNImageRequestHandler(cgImage: cgImage, options: [:])
    do {
        try handler.perform([request])
        for observation in (request.results ?? []).prefix(80) {
            guard let candidate = observation.topCandidates(1).first else { continue }
            let text = candidate.string.trimmingCharacters(in: .whitespacesAndNewlines)
            if !text.isEmpty && candidate.confidence >= 0.25 {
                output.texts.append(TextItem(text: text, confidence: candidate.confidence, page: page))
            }
        }
    } catch {
        output.warnings.append("OCR failed: \(error.localizedDescription)")
    }
}

func classifyImage(_ cgImage: CGImage, output: inout Output) {
    let request = VNClassifyImageRequest()
    let handler = VNImageRequestHandler(cgImage: cgImage, options: [:])
    do {
        try handler.perform([request])
        for observation in (request.results ?? []).prefix(8) {
            if observation.confidence >= 0.10 {
                output.classifications.append(ClassItem(label: observation.identifier, confidence: observation.confidence))
            }
        }
    } catch {
        output.warnings.append("Image classification failed: \(error.localizedDescription)")
    }
}

func analyzeImage(url: URL, output: inout Output) {
    guard let image = NSImage(contentsOf: url), let cg = cgImage(from: image) else {
        output.warnings.append("AppKit/ImageIO could not decode the image.")
        return
    }
    output.width = cg.width
    output.height = cg.height
    recognizeText(in: cg, page: nil, output: &output)
    classifyImage(cg, output: &output)
}

let arguments = Array(CommandLine.arguments.dropFirst())
let path = arguments.first ?? ""
var output = Output()
let url = URL(fileURLWithPath: path)
let ext = url.pathExtension.lowercased()
if ext == "pdf" {
    output.backend = "pdf-processing-refused"
    output.warnings.append("PDF files require the dedicated contained PDF helper.")
} else {
    analyzeImage(url: url, output: &output)
}

var seenTexts = Set<String>()
output.texts = output.texts.filter { item in
    let key = "\(item.page ?? 0):\(item.text)"
    if seenTexts.contains(key) { return false }
    seenTexts.insert(key)
    return true
}

let encoder = JSONEncoder()
if let data = try? encoder.encode(output), let json = String(data: data, encoding: .utf8) {
    print(json)
} else {
    print("{\"backend\":\"apple-vision-local\",\"warnings\":[\"JSON encoding failed\"]}")
}
