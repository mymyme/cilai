import AppKit
import Foundation
import PDFKit

func fail(_ reason: String) -> Never {
    FileHandle.standardError.write(Data((reason + "\n").utf8))
    exit(1)
}

let arguments = CommandLine.arguments
guard arguments.count >= 3 else {
    fail("pdf_renderer_arguments_invalid")
}

let mode = arguments[1]
let fileURL = URL(fileURLWithPath: arguments[2])
guard let document = PDFDocument(url: fileURL), document.pageCount > 0 else {
    fail("pdf_document_invalid")
}

if mode == "count" {
    print(document.pageCount)
    exit(0)
}

guard mode == "render", arguments.count == 5,
      let pageNumber = Int(arguments[3]),
      let maxWidth = Double(arguments[4]),
      pageNumber > 0,
      pageNumber <= document.pageCount,
      maxWidth >= 320,
      maxWidth <= 2400,
      let page = document.page(at: pageNumber - 1) else {
    fail("pdf_page_request_invalid")
}

let bounds = page.bounds(for: .mediaBox)
guard bounds.width > 0, bounds.height > 0 else {
    fail("pdf_page_geometry_invalid")
}
let targetSize = NSSize(width: maxWidth, height: maxWidth * bounds.height / bounds.width)
let image = page.thumbnail(of: targetSize, for: .mediaBox)
guard let tiff = image.tiffRepresentation,
      let bitmap = NSBitmapImageRep(data: tiff),
      let png = bitmap.representation(using: .png, properties: [:]) else {
    fail("pdf_page_render_failed")
}
FileHandle.standardOutput.write(png)
