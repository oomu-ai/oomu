import Foundation
import AppKit
import PDFKit

private let backendIdentity = "oomu-artifact-pdf-helper/apple-pdfkit-v1"

struct RenderOutput: Codable {
    let backend: String
    let page_count: Int
    let page_files: [String]
    let warnings: [String]
}

struct Metadata: Decodable {
    let title: String
    let subtitle: String
    let author: String
    let subject: String
}

struct Theme: Decodable {
    let fontFamily: String
    let bodySizePt: Double
    let titleSizePt: Double
}

struct Section: Decodable {
    let heading: String
    let pageBreakBefore: Bool
    let blocks: [Block]
}

struct ArtifactDocument: Decodable {
    let metadata: Metadata
    let theme: Theme
    let header: String?
    let footer: String?
    let sections: [Section]
}

enum Block: Decodable {
    case paragraph(String)
    case list(Bool, [String])
    case table([String], [[String]], String)
    case callout(String, String)
    case citation(String, String)
    case pageBreak

    enum Keys: String, CodingKey {
        case type, text, ordered, items, headers, rows, caption, label, url
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: Keys.self)
        switch try values.decode(String.self, forKey: .type) {
        case "paragraph":
            self = .paragraph(try values.decode(String.self, forKey: .text))
        case "list":
            self = .list(
                try values.decode(Bool.self, forKey: .ordered),
                try values.decode([String].self, forKey: .items)
            )
        case "table":
            self = .table(
                try values.decode([String].self, forKey: .headers),
                try values.decode([[String]].self, forKey: .rows),
                try values.decodeIfPresent(String.self, forKey: .caption) ?? ""
            )
        case "callout":
            self = .callout(
                try values.decode(String.self, forKey: .label),
                try values.decode(String.self, forKey: .text)
            )
        case "citation":
            self = .citation(
                try values.decode(String.self, forKey: .label),
                try values.decode(String.self, forKey: .url)
            )
        case "page_break":
            self = .pageBreak
        default:
            throw DecodingError.dataCorruptedError(
                forKey: .type,
                in: values,
                debugDescription: "Unsupported artifact block"
            )
        }
    }
}

func buildPdf(input: String, output: String) -> Bool {
    guard let data = FileManager.default.contents(atPath: input),
          let document = try? JSONDecoder().decode(ArtifactDocument.self, from: data) else {
        return false
    }
    var mediaBox = CGRect(x: 0, y: 0, width: 612, height: 792)
    let metadata: [CFString: Any] = [
        kCGPDFContextTitle: document.metadata.title,
        kCGPDFContextAuthor: document.metadata.author,
        kCGPDFContextSubject: document.metadata.subject,
        kCGPDFContextCreator: backendIdentity,
    ]
    guard let consumer = CGDataConsumer(url: URL(fileURLWithPath: output) as CFURL),
          let context = CGContext(
              consumer: consumer,
              mediaBox: &mediaBox,
              metadata as CFDictionary
          ) else {
        return false
    }

    let bodyFont = NSFont(
        name: document.theme.fontFamily,
        size: document.theme.bodySizePt
    ) ?? NSFont.systemFont(ofSize: document.theme.bodySizePt)
    let boldFont = NSFontManager.shared.convert(bodyFont, toHaveTrait: .boldFontMask)
    let titleFont = NSFont(
        name: document.theme.fontFamily,
        size: document.theme.titleSizePt
    ).map {
        NSFontManager.shared.convert($0, toHaveTrait: .boldFontMask)
    } ?? NSFont.boldSystemFont(ofSize: document.theme.titleSizePt)
    let headingFont = NSFontManager.shared.convert(bodyFont, toHaveTrait: .boldFontMask)
    let paragraph = NSMutableParagraphStyle()
    paragraph.lineSpacing = 2
    paragraph.paragraphSpacing = 6
    let bodyAttributes: [NSAttributedString.Key: Any] = [
        .font: bodyFont,
        .foregroundColor: NSColor.black,
        .paragraphStyle: paragraph,
    ]
    let boldAttributes: [NSAttributedString.Key: Any] = [
        .font: boldFont,
        .foregroundColor: NSColor.black,
        .paragraphStyle: paragraph,
    ]

    func tableAttributes(
        font: NSFont,
        color: NSColor = .black,
        lineBreakMode: NSLineBreakMode = .byWordWrapping
    ) -> [NSAttributedString.Key: Any] {
        let style = NSMutableParagraphStyle()
        style.lineSpacing = 1
        style.lineBreakMode = lineBreakMode
        return [
            .font: font,
            .foregroundColor: color,
            .paragraphStyle: style,
        ]
    }

    func tableFontSize(columnCount: Int) -> CGFloat {
        let reduction: CGFloat
        switch columnCount {
        case 0...4: reduction = 0
        case 5...6: reduction = 1
        case 7...8: reduction = 1.75
        default: reduction = 2.5
        }
        return max(8, bodyFont.pointSize - reduction)
    }

    func isCompactTableValue(_ value: String) -> Bool {
        let compactCharacters = CharacterSet(charactersIn: "0123456789.,%+$-() ")
        return !value.isEmpty
            && value.unicodeScalars.allSatisfy { compactCharacters.contains($0) }
    }

    func tableColumnWidths(
        headers: [String],
        rows: [[String]],
        columnCount: Int,
        font: NSFont
    ) -> [CGFloat] {
        guard columnCount > 0 else { return [] }
        let availableWidth: CGFloat = 468
        let bold = NSFontManager.shared.convert(font, toHaveTrait: .boldFontMask)
        var minimumWidths = Array(repeating: CGFloat(28), count: columnCount)
        var scores = Array(repeating: CGFloat(1), count: columnCount)
        for column in 0..<columnCount {
            let header = column < headers.count ? headers[column] : ""
            let headerWidth = ceil((header as NSString).size(withAttributes: [.font: bold]).width)
            let headerTokenWidth = header.components(separatedBy: .whitespacesAndNewlines)
                .filter { !$0.isEmpty }
                .map { ceil(($0 as NSString).size(withAttributes: [.font: bold]).width) }
                .max() ?? 0
            minimumWidths[column] = max(28, min(64, headerTokenWidth + 10))
            let values = rows.compactMap { column < $0.count ? $0[column] : nil }
            let nonemptyValues = values.filter { !$0.isEmpty }
            let bodyWidths = nonemptyValues.map {
                ceil(($0 as NSString).size(withAttributes: [.font: font]).width)
            }
            let widestBody = bodyWidths.max() ?? 0
            let averageBody = bodyWidths.isEmpty
                ? 0
                : bodyWidths.reduce(0, +) / CGFloat(bodyWidths.count)
            let compactBody = !nonemptyValues.isEmpty
                && nonemptyValues.allSatisfy({ isCompactTableValue($0) })
            if compactBody {
                minimumWidths[column] = max(
                    minimumWidths[column],
                    min(58, widestBody + 10)
                )
            }
            var score = max(
                min(headerWidth * 0.72, 110),
                min(widestBody * 0.7 + averageBody * 0.3, 180)
            )
            if compactBody {
                score *= 0.58
            }
            scores[column] = max(score, 12)
        }
        let minimumTotal = minimumWidths.reduce(0, +)
        if minimumTotal > availableWidth {
            let floorWidth: CGFloat = 24
            let reducible = max(minimumTotal - floorWidth * CGFloat(columnCount), 1)
            let requiredReduction = minimumTotal - availableWidth
            minimumWidths = minimumWidths.map {
                $0 - requiredReduction * (($0 - floorWidth) / reducible)
            }
        }
        let distributable = max(0, availableWidth - minimumWidths.reduce(0, +))
        let scoreTotal = max(scores.reduce(0, +), 1)
        var widths = scores.enumerated().map {
            minimumWidths[$0.offset] + distributable * ($0.element / scoreTotal)
        }
        let difference = availableWidth - widths.reduce(0, +)
        widths[columnCount - 1] += difference
        return widths
    }

    func longestTokenWidth(
        _ value: String,
        attributes: [NSAttributedString.Key: Any]
    ) -> CGFloat {
        value.components(separatedBy: .whitespacesAndNewlines)
            .filter { !$0.isEmpty }
            .map { ceil(($0 as NSString).size(withAttributes: attributes).width) }
            .max() ?? 0
    }

    func fittedTableAttributes(
        _ value: String,
        base: [NSAttributedString.Key: Any],
        width: CGFloat
    ) -> [NSAttributedString.Key: Any] {
        guard longestTokenWidth(value, attributes: base) > width else { return base }
        var fitted = base
        let style = (base[.paragraphStyle] as? NSParagraphStyle)?.mutableCopy()
            as? NSMutableParagraphStyle ?? NSMutableParagraphStyle()
        style.lineBreakMode = .byCharWrapping
        fitted[.paragraphStyle] = style
        return fitted
    }
    var pageNumber = 0
    var y: CGFloat = 72

    func beginPage() {
        context.beginPDFPage(nil)
        pageNumber += 1
        y = 72
        let graphics = NSGraphicsContext(cgContext: context, flipped: false)
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = graphics
        if let header = document.header, !header.isEmpty {
            (header as NSString).draw(
                at: CGPoint(x: 72, y: 766),
                withAttributes: [
                    .font: NSFont.systemFont(ofSize: 8),
                    .foregroundColor: NSColor.gray,
                ]
            )
        }
        if let footer = document.footer, !footer.isEmpty {
            (footer as NSString).draw(
                at: CGPoint(x: 72, y: 22),
                withAttributes: [
                    .font: NSFont.systemFont(ofSize: 8),
                    .foregroundColor: NSColor.gray,
                ]
            )
        }
        (String(pageNumber) as NSString).draw(
            at: CGPoint(x: 520, y: 22),
            withAttributes: [
                .font: NSFont.systemFont(ofSize: 8),
                .foregroundColor: NSColor.gray,
            ]
        )
    }

    func endPage() {
        NSGraphicsContext.restoreGraphicsState()
        context.endPDFPage()
    }

    func measure(
        _ text: String,
        _ attributes: [NSAttributedString.Key: Any],
        width: CGFloat = 468
    ) -> CGFloat {
        ceil((text as NSString).boundingRect(
            with: CGSize(width: width, height: 10_000),
            options: [.usesLineFragmentOrigin, .usesFontLeading],
            attributes: attributes
        ).height)
    }

    func ensure(_ height: CGFloat) {
        if y + height > 730 {
            endPage()
            beginPage()
        }
    }

    func draw(
        _ text: String,
        attributes: [NSAttributedString.Key: Any],
        indent: CGFloat = 0,
        spacing: CGFloat = 8
    ) {
        let width = 468 - indent
        let height = max(measure(text, attributes, width: width), 14)
        let pageBodyHeight: CGFloat = 730 - 72
        if height <= pageBodyHeight {
            ensure(height + spacing)
            (text as NSString).draw(
                with: CGRect(
                    x: 72 + indent,
                    y: 792 - y - height,
                    width: width,
                    height: height
                ),
                options: [.usesLineFragmentOrigin, .usesFontLeading],
                attributes: attributes
            )
            y += height + spacing
            return
        }

        // TextKit determines the exact complete glyph range that fits on each
        // page. The previous implementation moved an oversized paragraph to a
        // fresh page and then drew one rectangle taller than the media box,
        // silently clipping every later line. Flowing the same attributed text
        // through bounded page containers keeps every character visible.
        var remaining = NSAttributedString(string: text, attributes: attributes)
        while remaining.length > 0 {
            let availableHeight = 730 - y
            if availableHeight < 14 {
                endPage()
                beginPage()
                continue
            }
            let storage = NSTextStorage(attributedString: remaining)
            let layout = NSLayoutManager()
            let container = NSTextContainer(
                containerSize: CGSize(width: width, height: availableHeight)
            )
            container.lineFragmentPadding = 0
            container.lineBreakMode = .byWordWrapping
            layout.addTextContainer(container)
            storage.addLayoutManager(layout)
            layout.ensureLayout(for: container)
            let glyphRange = layout.glyphRange(for: container)
            let characterRange = layout.characterRange(
                forGlyphRange: glyphRange,
                actualGlyphRange: nil
            )
            if characterRange.length == 0 {
                endPage()
                beginPage()
                continue
            }
            let segment = remaining.attributedSubstring(from: characterRange)
            let segmentHeight = max(ceil(layout.usedRect(for: container).height), 14)
            segment.draw(
                with: CGRect(
                    x: 72 + indent,
                    y: 792 - y - segmentHeight,
                    width: width,
                    height: segmentHeight
                ),
                options: [.usesLineFragmentOrigin, .usesFontLeading],
                context: nil
            )
            y += segmentHeight
            let nextLocation = characterRange.location + characterRange.length
            if nextLocation >= remaining.length {
                remaining = NSAttributedString()
            } else {
                remaining = remaining.attributedSubstring(
                    from: NSRange(
                        location: nextLocation,
                        length: remaining.length - nextLocation
                    )
                )
                endPage()
                beginPage()
            }
        }
        y += spacing
    }

    beginPage()
    draw(
        document.metadata.title,
        attributes: [.font: titleFont, .foregroundColor: NSColor.black],
        spacing: 14
    )
    if !document.metadata.subtitle.isEmpty {
        draw(document.metadata.subtitle, attributes: bodyAttributes, spacing: 16)
    }
    for section in document.sections {
        if section.pageBreakBefore && y > 80 {
            endPage()
            beginPage()
        }
        // A section heading must read as a new thought, even when the previous
        // section continues onto the same page. Four points of residual list
        // spacing rendered as a visual collision in the executive PDF.
        if y > 80 {
            y += 10
        }
        if section.heading.caseInsensitiveCompare(document.metadata.title) != .orderedSame {
            draw(
                section.heading,
                attributes: [
                    .font: headingFont.withSize(17),
                    .foregroundColor: NSColor.black,
                ],
                spacing: 10
            )
        }
        for block in section.blocks {
            switch block {
            case .paragraph(let text):
                draw(text, attributes: bodyAttributes)
            case .list(let ordered, let items):
                for (index, item) in items.enumerated() {
                    draw(
                        "\(ordered ? "\(index + 1)." : "•") \(item)",
                        attributes: bodyAttributes,
                        indent: 14,
                        spacing: 4
                    )
                }
            case .table(let headers, let rows, let caption):
                if !caption.isEmpty {
                    draw(caption, attributes: boldAttributes, spacing: 4)
                }
                let columnCount = max(headers.count, rows.map(\.count).max() ?? 0)
                guard columnCount > 0 else { continue }
                let fontSize = tableFontSize(columnCount: columnCount)
                let tableFont = bodyFont.withSize(fontSize)
                let tableBoldFont = NSFontManager.shared.convert(
                    tableFont,
                    toHaveTrait: .boldFontMask
                )
                let tableBodyAttributes = tableAttributes(font: tableFont)
                let tableHeaderAttributes = tableAttributes(font: tableBoldFont)
                let columnWidths = tableColumnWidths(
                    headers: headers,
                    rows: rows,
                    columnCount: columnCount,
                    font: tableFont
                )
                let horizontalPadding: CGFloat = 5
                let verticalPadding: CGFloat = 5

                func measuredTableRowHeight(
                    _ row: [String],
                    attributes: [NSAttributedString.Key: Any],
                    minimum: CGFloat
                ) -> CGFloat {
                    var height = minimum
                    for column in 0..<columnCount {
                        let value = column < row.count ? row[column] : ""
                        let textWidth = max(1, columnWidths[column] - horizontalPadding * 2)
                        let fitted = fittedTableAttributes(
                            value,
                            base: attributes,
                            width: textWidth
                        )
                        height = max(
                            height,
                            measure(value, fitted, width: textWidth) + verticalPadding * 2
                        )
                    }
                    return ceil(height)
                }

                func drawTableRow(
                    _ row: [String],
                    attributes: [NSAttributedString.Key: Any],
                    height: CGFloat,
                    isHeader: Bool
                ) {
                    var x: CGFloat = 72
                    for column in 0..<columnCount {
                        let value = column < row.count ? row[column] : ""
                        let rect = CGRect(
                            x: x,
                            y: 792 - y - height,
                            width: columnWidths[column],
                            height: height
                        )
                        context.setFillColor((isHeader
                            ? NSColor(calibratedWhite: 0.92, alpha: 1)
                            : NSColor.white).cgColor)
                        context.fill(rect)
                        context.setStrokeColor(NSColor.lightGray.cgColor)
                        context.stroke(rect)
                        let textRect = rect.insetBy(
                            dx: horizontalPadding,
                            dy: verticalPadding
                        )
                        let fitted = fittedTableAttributes(
                            value,
                            base: attributes,
                            width: textRect.width
                        )
                        (value as NSString).draw(
                            with: textRect,
                            options: [.usesLineFragmentOrigin, .usesFontLeading],
                            attributes: fitted
                        )
                        x += columnWidths[column]
                    }
                    y += height
                }

                let headerHeight = measuredTableRowHeight(
                    headers,
                    attributes: tableHeaderAttributes,
                    minimum: 28
                )
                let firstRowHeight = rows.first.map {
                    measuredTableRowHeight(
                        $0,
                        attributes: tableBodyAttributes,
                        minimum: 26
                    )
                } ?? 0
                ensure(headerHeight + firstRowHeight)
                drawTableRow(
                    headers,
                    attributes: tableHeaderAttributes,
                    height: headerHeight,
                    isHeader: true
                )
                for row in rows {
                    let rowHeight = measuredTableRowHeight(
                        row,
                        attributes: tableBodyAttributes,
                        minimum: 26
                    )
                    if y + rowHeight > 730 {
                        endPage()
                        beginPage()
                        drawTableRow(
                            headers,
                            attributes: tableHeaderAttributes,
                            height: headerHeight,
                            isHeader: true
                        )
                    }
                    drawTableRow(
                        row,
                        attributes: tableBodyAttributes,
                        height: rowHeight,
                        isHeader: false
                    )
                }
                y += 8
            case .callout(let label, let text):
                draw(label, attributes: boldAttributes, indent: 10, spacing: 2)
                draw(text, attributes: bodyAttributes, indent: 10)
            case .citation(let label, let url):
                let value = "\(label) - \(url)"
                let height = max(measure(value, bodyAttributes), 14)
                ensure(height + 8)
                let rect = CGRect(x: 72, y: 792 - y - height, width: 468, height: height)
                (value as NSString).draw(
                    with: rect,
                    options: [.usesLineFragmentOrigin],
                    attributes: [.font: bodyFont, .foregroundColor: NSColor.systemBlue]
                )
                if let link = URL(string: url) {
                    context.setURL(link as CFURL, for: rect)
                }
                y += height + 8
            case .pageBreak:
                endPage()
                beginPage()
            }
        }
    }
    endPage()
    context.closePDF()
    return FileManager.default.fileExists(atPath: output)
}

func pngData(from image: NSImage) -> Data? {
    guard let tiff = image.tiffRepresentation,
          let bitmap = NSBitmapImageRep(data: tiff) else {
        return nil
    }
    return bitmap.representation(using: .png, properties: [:])
}

func renderPdf(input: String, outputDirectory: String) -> RenderOutput {
    let manager = FileManager.default
    let outputUrl = URL(fileURLWithPath: outputDirectory, isDirectory: true)
    do {
        try manager.createDirectory(at: outputUrl, withIntermediateDirectories: true)
    } catch {
        return RenderOutput(
            backend: backendIdentity,
            page_count: 0,
            page_files: [],
            warnings: ["Unable to create private render directory."]
        )
    }
    guard let document = PDFDocument(url: URL(fileURLWithPath: input)),
          document.pageCount > 0,
          document.pageCount <= 128 else {
        return RenderOutput(
            backend: backendIdentity,
            page_count: 0,
            page_files: [],
            warnings: ["PDFKit rejected the PDF or page-count limit."]
        )
    }
    var files: [String] = []
    var warnings: [String] = []
    for index in 0..<document.pageCount {
        guard let page = document.page(at: index) else {
            warnings.append("Missing page \(index + 1).")
            continue
        }
        let bounds = page.bounds(for: .mediaBox)
        let scale = min(2.0, 1800.0 / max(bounds.width, bounds.height))
        let size = NSSize(
            width: max(1, bounds.width * scale),
            height: max(1, bounds.height * scale)
        )
        let image = page.thumbnail(of: size, for: .mediaBox)
        guard let data = pngData(from: image), !data.isEmpty else {
            warnings.append("Page \(index + 1) PNG encoding failed.")
            continue
        }
        let path = outputUrl.appendingPathComponent(
            String(format: "page-%03d.png", index + 1)
        )
        do {
            try data.write(to: path, options: .atomic)
            files.append(path.path)
        } catch {
            warnings.append("Page \(index + 1) could not be written.")
        }
    }
    return RenderOutput(
        backend: backendIdentity,
        page_count: document.pageCount,
        page_files: files,
        warnings: warnings
    )
}

private let workbookRendererIdentity = "oomu-artifact-pdf-helper/apple-pdfkit-v1+appkit-sheet-v1"

struct WorkbookPreviewDocument: Decodable {
    let title: String
    let formats: [WorkbookPreviewFormat]
    let worksheets: [WorkbookPreviewSheet]
}

struct WorkbookPreviewFormat: Decodable {
    let formatId: String
    let fillColor: String?
    let wrapText: Bool
}

struct WorkbookPreviewSheet: Decodable {
    let sheetId: String
    let name: String
    let bounds: WorkbookPreviewBounds
    let cells: [WorkbookPreviewCell]
    let columnWidths: [WorkbookPreviewColumn]
    let charts: [WorkbookPreviewChart]
}

struct WorkbookPreviewBounds: Decodable { let rowCount: Int; let columnCount: Int }
struct WorkbookPreviewColumn: Decodable { let column: String; let width: Double }
struct WorkbookPreviewAnchor: Decodable { let fromColumn: Int; let fromRow: Int; let toColumn: Int; let toRow: Int }
struct WorkbookPreviewSeries: Decodable { let name: String; let valueRange: String }
struct WorkbookPreviewChart: Decodable {
    let chartId: String
    let title: String
    let categoryRange: String
    let series: [WorkbookPreviewSeries]
    let anchor: WorkbookPreviewAnchor
}

struct WorkbookPreviewCell: Decodable {
    let address: String
    let value: WorkbookPreviewValue
    let formatId: String?
}

enum WorkbookPreviewValue: Decodable {
    case blank, text(String), number(Double), boolean(Bool), date(String), formula(String, String?, Double?)
    enum Keys: String, CodingKey { case kind, value, iso, expression, cachedValue }
    enum CachedKeys: String, CodingKey { case kind, value, code }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: Keys.self)
        switch try values.decode(String.self, forKey: .kind) {
        case "blank": self = .blank
        case "text": self = .text(try values.decode(String.self, forKey: .value))
        case "number": self = .number(try values.decode(Double.self, forKey: .value))
        case "boolean": self = .boolean(try values.decode(Bool.self, forKey: .value))
        case "date": self = .date(try values.decode(String.self, forKey: .iso))
        case "formula":
            var cachedDisplay: String?, cachedNumber: Double?
            if values.contains(.cachedValue), !(try values.decodeNil(forKey: .cachedValue)) {
                let nested = try values.nestedContainer(keyedBy: CachedKeys.self, forKey: .cachedValue)
                switch try nested.decode(String.self, forKey: .kind) {
                case "number": if let value = try? nested.decode(Double.self, forKey: .value) { cachedNumber = value; cachedDisplay = String(format: "%g", value) }
                case "text": cachedDisplay = try? nested.decode(String.self, forKey: .value)
                case "boolean": cachedDisplay = (try? nested.decode(Bool.self, forKey: .value)).map { $0 ? "TRUE" : "FALSE" }
                case "error": cachedDisplay = try? nested.decode(String.self, forKey: .code)
                default: cachedDisplay = nil
                }
            }
            self = .formula(try values.decode(String.self, forKey: .expression), cachedDisplay, cachedNumber)
        default: throw DecodingError.dataCorruptedError(forKey: .kind, in: values, debugDescription: "Unsupported workbook preview cell value")
        }
    }
    var display: String {
        switch self { case .blank: return ""; case .text(let value): return value; case .number(let value): return String(format: "%g", value); case .boolean(let value): return value ? "TRUE" : "FALSE"; case .date(let value): return value; case .formula(let expression, let cached, _): return cached ?? "=\(expression)" }
    }
    var number: Double? { switch self { case .number(let value): return value; case .formula(_, _, let cached): return cached; default: return nil } }
}

struct WorkbookNativeSheet: Codable { let sheet_id: String; let file: String; let width: Int; let height: Int }
struct WorkbookNativeWarning: Codable { let code: String; let sheet_id: String?; let range: String?; let chart_id: String?; let technical_detail: String }
struct WorkbookNativeOutput: Codable { let backend: String; let sheet_previews: [WorkbookNativeSheet]; let warnings: [WorkbookNativeWarning] }

func workbookAddress(_ raw: String) -> (row: Int, column: Int)? {
    let value = raw.replacingOccurrences(of: "$", with: "").uppercased()
    let letters = value.prefix { $0.isLetter }
    let digits = value.dropFirst(letters.count)
    guard !letters.isEmpty, let row = Int(digits), row > 0 else { return nil }
    var column = 0
    for scalar in letters.unicodeScalars { column = column * 26 + Int(scalar.value) - 64 }
    return (row, column)
}

func workbookRange(_ raw: String) -> [(row: Int, column: Int)] {
    let local = raw.split(separator: "!").last.map(String.init) ?? raw
    let ends = local.split(separator: ":").map(String.init)
    guard let start = workbookAddress(ends.first ?? ""), let end = workbookAddress(ends.count > 1 ? ends[1] : (ends.first ?? "")), end.row >= start.row, end.column >= start.column else { return [] }
    return (start.row...end.row).flatMap { row in (start.column...end.column).map { (row, $0) } }
}

func workbookColor(_ hex: String?) -> NSColor {
    guard let hex, hex.count == 6, let value = Int(hex, radix: 16) else { return .white }
    return NSColor(calibratedRed: CGFloat((value >> 16) & 255) / 255, green: CGFloat((value >> 8) & 255) / 255, blue: CGFloat(value & 255) / 255, alpha: 1)
}

func renderWorkbookSheet(_ document: WorkbookPreviewDocument, sheet: WorkbookPreviewSheet, index: Int, output: URL) -> (WorkbookNativeSheet?, [WorkbookNativeWarning]) {
    let canvas = NSSize(width: 1200, height: 800)
    let image = NSImage(size: canvas)
    var warnings: [WorkbookNativeWarning] = []
    let formats = Dictionary(uniqueKeysWithValues: document.formats.map { ($0.formatId, $0) })
    let cells = Dictionary(uniqueKeysWithValues: sheet.cells.compactMap { cell in workbookAddress(cell.address).map { ("\($0.row):\($0.column)", cell) } })
    let widths = Dictionary(uniqueKeysWithValues: sheet.columnWidths.compactMap { width in workbookAddress(width.column + "1").map { ($0.column, width.width) } })
    image.lockFocus()
    NSColor(calibratedWhite: 0.98, alpha: 1).setFill(); NSRect(origin: .zero, size: canvas).fill()
    NSColor(calibratedRed: 0.12, green: 0.16, blue: 0.22, alpha: 1).setFill(); NSRect(x: 0, y: 752, width: 1200, height: 48).fill()
    let headerAttributes: [NSAttributedString.Key: Any] = [.font: NSFont.systemFont(ofSize: 16, weight: .semibold), .foregroundColor: NSColor.white]
    ("\(document.title) / \(sheet.name)" as NSString).draw(in: NSRect(x: 18, y: 766, width: 1164, height: 22), withAttributes: headerAttributes)
    let bodyAttributes: [NSAttributedString.Key: Any] = [.font: NSFont.systemFont(ofSize: 11), .foregroundColor: NSColor.textColor]
    let labelAttributes: [NSAttributedString.Key: Any] = [.font: NSFont.systemFont(ofSize: 10, weight: .medium), .foregroundColor: NSColor.darkGray]
    var columns: [(Int, CGFloat, CGFloat)] = []
    var x: CGFloat = 42
    for column in 1...min(sheet.bounds.columnCount, 20) {
        let width = CGFloat(min(240, max(42, (widths[column] ?? 12) * 7 + 10)))
        if x + width > 1200 { break }
        columns.append((column, x, width)); x += width
    }
    var rowHeights: [Int: CGFloat] = [:]
    for row in 1...min(sheet.bounds.rowCount, 40) {
        var height: CGFloat = 24
        for (column, _, width) in columns {
            guard let cell = cells["\(row):\(column)"], let id = cell.formatId, formats[id]?.wrapText == true else { continue }
            let measured = (cell.value.display as NSString).boundingRect(with: NSSize(width: width - 8, height: 140), options: [.usesLineFragmentOrigin, .usesFontLeading], attributes: bodyAttributes).height + 8
            height = max(height, min(140, ceil(measured)))
        }
        rowHeights[row] = height
    }
    var top: CGFloat = 96
    for (column, start, width) in columns {
        NSColor(calibratedWhite: 0.91, alpha: 1).setFill(); NSRect(x: start, y: 704, width: width, height: 24).fill(); NSColor.lightGray.setStroke(); NSBezierPath(rect: NSRect(x: start, y: 704, width: width, height: 24)).stroke()
        (columnLabel(column) as NSString).draw(in: NSRect(x: start + 5, y: 709, width: width - 10, height: 14), withAttributes: labelAttributes)
    }
    for row in 1...min(sheet.bounds.rowCount, 40) {
        let height = rowHeights[row] ?? 24
        if top + height > 800 { warnings.append(WorkbookNativeWarning(code: "preview_truncated", sheet_id: sheet.sheetId, range: nil, chart_id: nil, technical_detail: "Preview shows the leading visible region.")); break }
        let y = 800 - top - height
        NSColor(calibratedWhite: 0.95, alpha: 1).setFill(); NSRect(x: 0, y: y, width: 42, height: height).fill()
        (String(row) as NSString).draw(in: NSRect(x: 6, y: y + max(4, (height - 14) / 2), width: 32, height: 14), withAttributes: labelAttributes)
        for (column, start, width) in columns {
            let cell = cells["\(row):\(column)"]
            workbookColor(cell?.formatId.flatMap { formats[$0]?.fillColor }).setFill(); NSRect(x: start, y: y, width: width, height: height).fill(); NSColor.lightGray.setStroke(); NSBezierPath(rect: NSRect(x: start, y: y, width: width, height: height)).stroke()
            guard let cell else { continue }
            let rect = NSRect(x: start + 4, y: y + 4, width: width - 8, height: height - 8)
            let wrap = cell.formatId.flatMap { formats[$0]?.wrapText } ?? false
            let options: NSString.DrawingOptions = wrap ? [.usesLineFragmentOrigin, .usesFontLeading] : [.truncatesLastVisibleLine]
            if !wrap && (cell.value.display as NSString).size(withAttributes: bodyAttributes).width > rect.width { warnings.append(WorkbookNativeWarning(code: "column_content_clipped", sheet_id: sheet.sheetId, range: cell.address, chart_id: nil, technical_detail: "Cell text is wider than its configured column.")) }
            (cell.value.display as NSString).draw(with: rect, options: options, attributes: bodyAttributes)
        }
        top += height
    }
    for chart in sheet.charts {
        let chartX = CGFloat(42 + chart.anchor.fromColumn * 70), chartY = CGFloat(max(20, 800 - 72 - chart.anchor.toRow * 24))
        let chartWidth = CGFloat(max(120, (chart.anchor.toColumn - chart.anchor.fromColumn) * 70)), chartHeight = CGFloat(max(100, (chart.anchor.toRow - chart.anchor.fromRow) * 24))
        let boundedWidth = min(chartWidth, 1150 - chartX), boundedHeight = min(chartHeight, 680 - chartY)
        if boundedWidth <= 0 || boundedHeight <= 0 { warnings.append(WorkbookNativeWarning(code: "preview_unavailable", sheet_id: sheet.sheetId, range: nil, chart_id: chart.chartId, technical_detail: "Chart anchor falls outside preview bounds.")); continue }
        let rect = NSRect(x: chartX, y: chartY, width: boundedWidth, height: boundedHeight)
        NSColor.white.setFill(); rect.fill(); NSColor.darkGray.setStroke(); NSBezierPath(rect: rect).stroke(); (chart.title as NSString).draw(in: rect.insetBy(dx: 10, dy: 8), withAttributes: labelAttributes)
        let values = chart.series.first.map { series in workbookRange(series.valueRange).compactMap { cells["\($0.row):\($0.column)"]?.value.number } } ?? []
        if values.isEmpty { warnings.append(WorkbookNativeWarning(code: "chart_data_missing", sheet_id: sheet.sheetId, range: chart.series.first?.valueRange, chart_id: chart.chartId, technical_detail: "Chart has no renderable numeric source values.")) }
        let maximum = max(values.map(abs).max() ?? 1, 1)
        for (bar, value) in values.prefix(12).enumerated() {
            let width = max(8, (rect.width - 30) / CGFloat(max(values.count, 1)) - 5), height = max(1, (rect.height - 48) * CGFloat(abs(value) / maximum))
            NSColor.systemBlue.setFill(); NSRect(x: rect.minX + 15 + CGFloat(bar) * (width + 5), y: rect.minY + 15, width: width, height: height).fill()
        }
    }
    image.unlockFocus()
    guard let data = pngData(from: image) else { return (nil, [WorkbookNativeWarning(code: "preview_unavailable", sheet_id: sheet.sheetId, range: nil, chart_id: nil, technical_detail: "AppKit PNG encoding failed.")]) }
    let path = output.appendingPathComponent(String(format: "sheet-%03d.png", index + 1))
    do { try data.write(to: path, options: .atomic) } catch { return (nil, [WorkbookNativeWarning(code: "preview_unavailable", sheet_id: sheet.sheetId, range: nil, chart_id: nil, technical_detail: "AppKit preview write failed.")]) }
    return (WorkbookNativeSheet(sheet_id: sheet.sheetId, file: path.path, width: 1200, height: 800), warnings)
}

func columnLabel(_ column: Int) -> String { var value = column; var result = ""; while value > 0 { result = String(UnicodeScalar(65 + (value - 1) % 26)!) + result; value = (value - 1) / 26 }; return result }

func renderWorkbookPreview(input: String, outputDirectory: String) -> WorkbookNativeOutput {
    let output = URL(fileURLWithPath: outputDirectory, isDirectory: true)
    guard let data = FileManager.default.contents(atPath: input), let workbook = try? JSONDecoder().decode(WorkbookPreviewDocument.self, from: data), workbook.worksheets.count <= 1024 else { return WorkbookNativeOutput(backend: workbookRendererIdentity, sheet_previews: [], warnings: [WorkbookNativeWarning(code: "preview_unavailable", sheet_id: nil, range: nil, chart_id: nil, technical_detail: "Workbook preview input is invalid.")]) }
    do { try FileManager.default.createDirectory(at: output, withIntermediateDirectories: false) } catch { return WorkbookNativeOutput(backend: workbookRendererIdentity, sheet_previews: [], warnings: [WorkbookNativeWarning(code: "preview_unavailable", sheet_id: nil, range: nil, chart_id: nil, technical_detail: "Private preview directory could not be created.")]) }
    var previews: [WorkbookNativeSheet] = [], warnings: [WorkbookNativeWarning] = []
    for (index, sheet) in workbook.worksheets.enumerated() { let rendered = renderWorkbookSheet(workbook, sheet: sheet, index: index, output: output); if let preview = rendered.0 { previews.append(preview) }; warnings.append(contentsOf: rendered.1) }
    return WorkbookNativeOutput(backend: workbookRendererIdentity, sheet_previews: previews, warnings: warnings)
}

private let presentationRendererIdentity = "oomu-artifact-pdf-helper/apple-pdfkit-v1+appkit-presentation-v1"

struct PresentationPreviewDocument: Decodable {
    let aspectRatio: String
    let theme: PresentationPreviewTheme
    let slides: [PresentationPreviewSlide]
    let citations: [PresentationPreviewCitation]
    let policy: PresentationPreviewPolicy
}
struct PresentationPreviewTheme: Decodable { let colors: PresentationPreviewColors; let fonts: PresentationPreviewFonts }
struct PresentationPreviewColors: Decodable { let dark: String; let light: String; let accent1: String }
struct PresentationPreviewFonts: Decodable { let heading: String; let body: String }
struct PresentationPreviewPolicy: Decodable { let minimumFontSizePt: Double; let minimumImageDpi: Int; let allowedFonts: [String] }
struct PresentationPreviewCitation: Decodable { let slideId: String; let objectId: String?; let sourceRef: String; let evidenceRef: String }
struct PresentationPreviewSlide: Decodable { let slideId: String; let title: String?; let elements: [PresentationPreviewElement]; let notes: PresentationPreviewNotes }
struct PresentationPreviewNotes: Decodable { let speakerNotes: String; let sourceRefs: [String] }
struct PresentationPreviewElement: Decodable {
    let objectId: String
    let frame: PresentationPreviewFrame
    let content: PresentationPreviewContent
    let provenance: [PresentationPreviewProvenance]
}
struct PresentationPreviewFrame: Decodable { let x: Int64; let y: Int64; let width: Int64; let height: Int64 }
struct PresentationPreviewProvenance: Decodable { let sourceRef: String; let evidenceRef: String }
struct PresentationPreviewTextBlock: Decodable { let paragraphs: [PresentationPreviewParagraph]; let verticalAlignment: String }
struct PresentationPreviewParagraph: Decodable { let runs: [PresentationPreviewRun]; let alignment: String; let bullet: Bool }
struct PresentationPreviewRun: Decodable { let text: String; let fontFamily: String; let fontSizePt: Double; let bold: Bool; let italic: Bool; let color: String }
struct PresentationPreviewImage: Decodable { let mediaType: String; let bytesBase64: String; let widthPx: Int; let heightPx: Int; let altText: String }
struct PresentationPreviewTable: Decodable { let rows: [[PresentationPreviewTextBlock]]; let headerRow: Bool }
struct PresentationPreviewChart: Decodable { let chartType: String; let title: String; let categories: [String]; let series: [PresentationPreviewSeries] }
struct PresentationPreviewSeries: Decodable { let name: String; let values: [Double] }

enum PresentationPreviewContent: Decodable {
    case text(PresentationPreviewTextBlock)
    case shape(String, String, String?, PresentationPreviewTextBlock?)
    case image(PresentationPreviewImage)
    case table(PresentationPreviewTable)
    case chart(PresentationPreviewChart)
    enum Keys: String, CodingKey { case kind, text, geometry, fillColor, lineColor, image, table, chart }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: Keys.self)
        switch try values.decode(String.self, forKey: .kind) {
        case "text_box": self = .text(try values.decode(PresentationPreviewTextBlock.self, forKey: .text))
        case "shape": self = .shape(try values.decode(String.self, forKey: .geometry), try values.decode(String.self, forKey: .fillColor), try values.decodeIfPresent(String.self, forKey: .lineColor), try values.decodeIfPresent(PresentationPreviewTextBlock.self, forKey: .text))
        case "image": self = .image(try values.decode(PresentationPreviewImage.self, forKey: .image))
        case "table": self = .table(try values.decode(PresentationPreviewTable.self, forKey: .table))
        case "chart": self = .chart(try values.decode(PresentationPreviewChart.self, forKey: .chart))
        default: throw DecodingError.dataCorruptedError(forKey: .kind, in: values, debugDescription: "Unsupported presentation element")
        }
    }
    var label: String { switch self { case .text: return "text"; case .shape: return "shape"; case .image: return "image"; case .table: return "table"; case .chart: return "chart" } }
}

struct PresentationNativeSlide: Codable { let slide_id: String; let file: String; let width: Int; let height: Int }
struct PresentationNativeWarning: Codable { let code: String; let slide_id: String?; let object_id: String?; let technical_detail: String }
struct PresentationNativeOutput: Codable { let backend: String; let slide_previews: [PresentationNativeSlide]; let warnings: [PresentationNativeWarning] }

func presentationColor(_ hex: String, fallback: NSColor = .black) -> NSColor {
    guard hex.count == 6, let value = Int(hex, radix: 16) else { return fallback }
    return NSColor(calibratedRed: CGFloat((value >> 16) & 255) / 255, green: CGFloat((value >> 8) & 255) / 255, blue: CGFloat(value & 255) / 255, alpha: 1)
}

func presentationRect(_ frame: PresentationPreviewFrame, canvas: NSSize, aspectRatio: String) -> NSRect {
    let emuWidth: CGFloat = aspectRatio == "4:3" ? 9_144_000 : 12_192_000
    let emuHeight: CGFloat = 6_858_000
    let sx = canvas.width / emuWidth, sy = canvas.height / emuHeight
    return NSRect(x: CGFloat(frame.x) * sx, y: canvas.height - CGFloat(frame.y + frame.height) * sy, width: CGFloat(frame.width) * sx, height: CGFloat(frame.height) * sy)
}

func attributedPresentationText(_ block: PresentationPreviewTextBlock, fallbackFont: String, warnings: inout [PresentationNativeWarning], slideId: String, objectId: String) -> NSAttributedString {
    let output = NSMutableAttributedString()
    for (paragraphIndex, paragraph) in block.paragraphs.enumerated() {
        if paragraphIndex > 0 { output.append(NSAttributedString(string: "\n")) }
        if paragraph.bullet { output.append(NSAttributedString(string: "• ")) }
        for run in paragraph.runs {
            var traits: NSFontTraitMask = []
            if run.bold { traits.insert(.boldFontMask) }
            if run.italic { traits.insert(.italicFontMask) }
            let requested = NSFont(name: run.fontFamily, size: CGFloat(run.fontSizePt))
            if requested == nil { warnings.append(PresentationNativeWarning(code: "missing_font", slide_id: slideId, object_id: objectId, technical_detail: "A requested font is unavailable to the local renderer.")) }
            let base = requested ?? NSFont(name: fallbackFont, size: CGFloat(run.fontSizePt)) ?? NSFont.systemFont(ofSize: CGFloat(run.fontSizePt))
            let font = traits.isEmpty ? base : NSFontManager.shared.convert(base, toHaveTrait: traits)
            let style = NSMutableParagraphStyle()
            style.alignment = paragraph.alignment == "center" ? .center : paragraph.alignment == "right" ? .right : .left
            output.append(NSAttributedString(string: run.text, attributes: [.font: font, .foregroundColor: presentationColor(run.color), .paragraphStyle: style]))
        }
    }
    return output
}

func presentationContrast(_ foreground: NSColor, _ background: NSColor) -> Double {
    func luminance(_ color: NSColor) -> Double {
        guard let rgb = color.usingColorSpace(.sRGB) else { return 1 }
        let channels = [rgb.redComponent, rgb.greenComponent, rgb.blueComponent].map { value -> Double in let v = Double(value); return v <= 0.03928 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4) }
        return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2]
    }
    let left = luminance(foreground), right = luminance(background)
    return (max(left, right) + 0.05) / (min(left, right) + 0.05)
}

func presentationTableHeaderBackground(_ cell: PresentationPreviewTextBlock) -> NSColor {
    guard
        let color = cell.paragraphs.first?.runs.first?.color,
        color.count == 6,
        let value = Int(color, radix: 16)
    else { return presentationColor("D9E2F3") }
    let red = (value >> 16) & 255
    let green = (value >> 8) & 255
    let blue = value & 255
    return red * 299 + green * 587 + blue * 114 >= 160_000
        ? presentationColor("17365D")
        : presentationColor("D9E2F3")
}

func drawPresentationText(_ block: PresentationPreviewTextBlock, in rect: NSRect, fallbackFont: String, background: NSColor, warnings: inout [PresentationNativeWarning], slideId: String, objectId: String) {
    let attributed = attributedPresentationText(block, fallbackFont: fallbackFont, warnings: &warnings, slideId: slideId, objectId: objectId)
    if attributed.string.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { warnings.append(PresentationNativeWarning(code: "empty_placeholder", slide_id: slideId, object_id: objectId, technical_detail: "A visible text area is empty.")); return }
    let measured = attributed.boundingRect(with: NSSize(width: max(1, rect.width), height: 10_000), options: [.usesLineFragmentOrigin, .usesFontLeading])
    if measured.height > rect.height + 1 || measured.width > rect.width + 1 { warnings.append(PresentationNativeWarning(code: "text_overflow", slide_id: slideId, object_id: objectId, technical_detail: "Text does not fit inside its editable frame.")) }
    if let first = block.paragraphs.first?.runs.first, presentationContrast(presentationColor(first.color), background) < 4.5 { warnings.append(PresentationNativeWarning(code: "contrast_failure", slide_id: slideId, object_id: objectId, technical_detail: "Text contrast is below the qualified threshold.")) }
    var target = rect
    if block.verticalAlignment == "middle" { target.origin.y += max(0, (rect.height - measured.height) / 2) }
    if block.verticalAlignment == "bottom" { target.origin.y += max(0, rect.height - measured.height) }
    NSGraphicsContext.saveGraphicsState(); NSBezierPath(rect: rect).addClip(); attributed.draw(with: target, options: [.usesLineFragmentOrigin, .usesFontLeading]); NSGraphicsContext.restoreGraphicsState()
}

func drawPresentationChart(_ chart: PresentationPreviewChart, in rect: NSRect, warnings: inout [PresentationNativeWarning], slideId: String, objectId: String) {
    NSColor(calibratedWhite: 0.98, alpha: 1).setFill(); rect.fill(); NSColor.lightGray.setStroke(); NSBezierPath(rect: rect).stroke()
    let titleAttributes: [NSAttributedString.Key: Any] = [.font: NSFont.systemFont(ofSize: 15, weight: .semibold), .foregroundColor: NSColor.textColor]
    (chart.title as NSString).draw(in: rect.insetBy(dx: 12, dy: 10), withAttributes: titleAttributes)
    guard !chart.categories.isEmpty, let series = chart.series.first, series.values.count == chart.categories.count else { warnings.append(PresentationNativeWarning(code: "broken_chart", slide_id: slideId, object_id: objectId, technical_detail: "Chart categories and values do not match.")); return }
    let maximum = max(series.values.map(abs).max() ?? 1, 1), plot = NSRect(x: rect.minX + 20, y: rect.minY + 20, width: rect.width - 40, height: rect.height - 58)
    if chart.chartType == "line" {
        let path = NSBezierPath()
        for (index, value) in series.values.enumerated() { let x = plot.minX + CGFloat(index) * plot.width / CGFloat(max(1, series.values.count - 1)); let y = plot.minY + CGFloat(abs(value) / maximum) * plot.height; index == 0 ? path.move(to: NSPoint(x: x, y: y)) : path.line(to: NSPoint(x: x, y: y)) }
        NSColor.systemBlue.setStroke(); path.lineWidth = 2; path.stroke()
    } else {
        let itemWidth = plot.width / CGFloat(max(1, series.values.count))
        for (index, value) in series.values.enumerated() { let height = max(1, CGFloat(abs(value) / maximum) * plot.height); NSColor.systemBlue.setFill(); NSRect(x: plot.minX + CGFloat(index) * itemWidth + 3, y: plot.minY, width: max(2, itemWidth - 6), height: height).fill() }
    }
}

func renderPresentationSlide(_ document: PresentationPreviewDocument, slide: PresentationPreviewSlide, index: Int, output: URL) -> (PresentationNativeSlide?, [PresentationNativeWarning]) {
    let canvas = document.aspectRatio == "4:3" ? NSSize(width: 960, height: 720) : NSSize(width: 1280, height: 720)
    let background = presentationColor(document.theme.colors.light, fallback: .white), image = NSImage(size: canvas)
    var warnings: [PresentationNativeWarning] = []
    image.lockFocus(); background.setFill(); NSRect(origin: .zero, size: canvas).fill()
    for element in slide.elements {
        let rect = presentationRect(element.frame, canvas: canvas, aspectRatio: document.aspectRatio)
        switch element.content {
        case .text(let block): drawPresentationText(block, in: rect, fallbackFont: document.theme.fonts.body, background: background, warnings: &warnings, slideId: slide.slideId, objectId: element.objectId)
        case .shape(let geometry, let fillHex, let lineHex, let block):
            let fill = presentationColor(fillHex), path: NSBezierPath
            if geometry == "ellipse" { path = NSBezierPath(ovalIn: rect) }
            else if geometry == "rounded_rectangle" { path = NSBezierPath(roundedRect: rect, xRadius: 12, yRadius: 12) }
            else if geometry == "triangle" { path = NSBezierPath(); path.move(to: NSPoint(x: rect.midX, y: rect.maxY)); path.line(to: NSPoint(x: rect.maxX, y: rect.minY)); path.line(to: NSPoint(x: rect.minX, y: rect.minY)); path.close() }
            else { path = NSBezierPath(rect: rect) }
            fill.setFill(); path.fill(); presentationColor(lineHex ?? fillHex).setStroke(); path.stroke()
            if let block { drawPresentationText(block, in: rect.insetBy(dx: 8, dy: 6), fallbackFont: document.theme.fonts.body, background: fill, warnings: &warnings, slideId: slide.slideId, objectId: element.objectId) }
        case .image(let source):
            guard let data = Data(base64Encoded: source.bytesBase64), let sourceImage = NSImage(data: data) else { warnings.append(PresentationNativeWarning(code: "missing_asset", slide_id: slide.slideId, object_id: element.objectId, technical_detail: "An image asset could not be decoded.")); continue }
            let frameInches = max(Double(element.frame.width) / 914_400, Double(element.frame.height) / 914_400), pixels = Double(min(source.widthPx, source.heightPx))
            if frameInches > 0 && pixels / frameInches < Double(document.policy.minimumImageDpi) { warnings.append(PresentationNativeWarning(code: "low_resolution_image", slide_id: slide.slideId, object_id: element.objectId, technical_detail: "An image is below the configured resolution threshold.")) }
            sourceImage.draw(in: rect, from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: [.interpolation: NSImageInterpolation.high])
        case .table(let table):
            guard !table.rows.isEmpty, let columnCount = table.rows.first?.count, columnCount > 0 else { warnings.append(PresentationNativeWarning(code: "empty_placeholder", slide_id: slide.slideId, object_id: element.objectId, technical_detail: "A table contains no visible cells.")); continue }
            let rowHeight = rect.height / CGFloat(table.rows.count), columnWidth = rect.width / CGFloat(columnCount)
            for (rowIndex, row) in table.rows.enumerated() { for (columnIndex, cell) in row.enumerated() { let cellRect = NSRect(x: rect.minX + CGFloat(columnIndex) * columnWidth, y: rect.maxY - CGFloat(rowIndex + 1) * rowHeight, width: columnWidth, height: rowHeight); let cellBackground = table.headerRow && rowIndex == 0 ? presentationTableHeaderBackground(cell) : NSColor.white; cellBackground.setFill(); cellRect.fill(); NSColor.lightGray.setStroke(); NSBezierPath(rect: cellRect).stroke(); drawPresentationText(cell, in: cellRect.insetBy(dx: 5, dy: 4), fallbackFont: document.theme.fonts.body, background: cellBackground, warnings: &warnings, slideId: slide.slideId, objectId: element.objectId) } }
        case .chart(let chart): drawPresentationChart(chart, in: rect, warnings: &warnings, slideId: slide.slideId, objectId: element.objectId)
        }
    }
    for left in 0..<slide.elements.count { for right in (left + 1)..<slide.elements.count {
        let a = slide.elements[left], b = slide.elements[right]
        if a.content.label == "text" && b.content.label == "text" && presentationRect(a.frame, canvas: canvas, aspectRatio: document.aspectRatio).intersects(presentationRect(b.frame, canvas: canvas, aspectRatio: document.aspectRatio)) { warnings.append(PresentationNativeWarning(code: "element_overlap", slide_id: slide.slideId, object_id: b.objectId, technical_detail: "Two editable text areas overlap.")) }
    } }
    for element in slide.elements where !element.provenance.isEmpty { for anchor in element.provenance {
        let cited = document.citations.contains { $0.slideId == slide.slideId && ($0.objectId == nil || $0.objectId == element.objectId) && $0.sourceRef == anchor.sourceRef && $0.evidenceRef == anchor.evidenceRef }
        if !cited { warnings.append(PresentationNativeWarning(code: "citation_omission", slide_id: slide.slideId, object_id: element.objectId, technical_detail: "A sourced object has no matching citation.")) }
    } }
    image.unlockFocus()
    guard let data = pngData(from: image), !data.isEmpty else { return (nil, [PresentationNativeWarning(code: "preview_unavailable", slide_id: slide.slideId, object_id: nil, technical_detail: "Slide PNG encoding failed.")]) }
    let path = output.appendingPathComponent(String(format: "slide-%03d.png", index + 1))
    do { try data.write(to: path, options: .atomic) } catch { return (nil, [PresentationNativeWarning(code: "preview_unavailable", slide_id: slide.slideId, object_id: nil, technical_detail: "Slide preview could not be written.")]) }
    return (PresentationNativeSlide(slide_id: slide.slideId, file: path.path, width: Int(canvas.width), height: Int(canvas.height)), warnings)
}

func renderPresentationPreview(input: String, outputDirectory: String) -> PresentationNativeOutput {
    let output = URL(fileURLWithPath: outputDirectory, isDirectory: true)
    guard let data = FileManager.default.contents(atPath: input), let document = try? JSONDecoder().decode(PresentationPreviewDocument.self, from: data), !document.slides.isEmpty, document.slides.count <= 1_000 else { return PresentationNativeOutput(backend: presentationRendererIdentity, slide_previews: [], warnings: [PresentationNativeWarning(code: "preview_unavailable", slide_id: nil, object_id: nil, technical_detail: "Presentation preview input is invalid.")]) }
    do { try FileManager.default.createDirectory(at: output, withIntermediateDirectories: false) } catch { return PresentationNativeOutput(backend: presentationRendererIdentity, slide_previews: [], warnings: [PresentationNativeWarning(code: "preview_unavailable", slide_id: nil, object_id: nil, technical_detail: "Private preview directory could not be created.")]) }
    var previews: [PresentationNativeSlide] = [], warnings: [PresentationNativeWarning] = []
    for (index, slide) in document.slides.enumerated() { let rendered = renderPresentationSlide(document, slide: slide, index: index, output: output); if let preview = rendered.0 { previews.append(preview) }; warnings.append(contentsOf: rendered.1) }
    return PresentationNativeOutput(backend: presentationRendererIdentity, slide_previews: previews, warnings: warnings)
}

let arguments = Array(CommandLine.arguments.dropFirst())
if arguments.first == "--probe-pdf-renderer" {
    print("{\"backend\":\"\(backendIdentity)\",\"available\":true}")
    exit(0)
}
if arguments.first == "--probe-workbook-renderer" {
    print("{\"backend\":\"\(workbookRendererIdentity)\",\"available\":true}")
    exit(0)
}
if arguments.first == "--probe-presentation-renderer" {
    print("{\"backend\":\"\(presentationRendererIdentity)\",\"available\":true}")
    exit(0)
}
if arguments.first == "--render-presentation-preview", arguments.count == 3 {
    let rendered = renderPresentationPreview(input: arguments[1], outputDirectory: arguments[2])
    if let data = try? JSONEncoder().encode(rendered), let json = String(data: data, encoding: .utf8) { print(json); exit(rendered.slide_previews.count > 0 && rendered.warnings.allSatisfy { $0.code != "preview_unavailable" } ? 0 : 1) }
    exit(1)
}
if arguments.first == "--render-workbook-preview", arguments.count == 3 {
    let rendered = renderWorkbookPreview(input: arguments[1], outputDirectory: arguments[2])
    if let data = try? JSONEncoder().encode(rendered), let json = String(data: data, encoding: .utf8) { print(json); exit(rendered.sheet_previews.count > 0 && rendered.warnings.allSatisfy { $0.code != "preview_unavailable" } ? 0 : 1) }
    exit(1)
}
if arguments.first == "--render-pdf", arguments.count == 3 {
    let rendered = renderPdf(input: arguments[1], outputDirectory: arguments[2])
    let encoder = JSONEncoder()
    if let data = try? encoder.encode(rendered),
       let json = String(data: data, encoding: .utf8) {
        print(json)
        exit(rendered.page_count == rendered.page_files.count ? 0 : 1)
    }
    print("{\"backend\":\"\(backendIdentity)\",\"page_count\":0,\"page_files\":[],\"warnings\":[\"JSON encoding failed\"]}")
    exit(1)
}
if arguments.first == "--build-artifact-pdf", arguments.count == 3 {
    exit(buildPdf(input: arguments[1], output: arguments[2]) ? 0 : 1)
}
exit(2)
