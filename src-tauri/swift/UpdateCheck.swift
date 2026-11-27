import Foundation

public typealias UpdateCallback = @convention(c) (
    UnsafeMutableRawPointer?, Int32, UnsafePointer<UInt8>?, Int,
    UnsafePointer<CChar>?, UnsafePointer<CChar>?, UnsafePointer<CChar>?
) -> Void

// State is written on URLSession's serial delegate queue and read after completion.
private final class ReleaseResponse: NSObject, URLSessionDataDelegate, @unchecked Sendable {
    let completed = DispatchSemaphore(value: 0)
    var response: HTTPURLResponse?
    var body = Data()
    var code: Int32 = -3
    private var oversized = false
    private let limit = 256 * 1024

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse,
                    completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        self.response = response as? HTTPURLResponse
        oversized = response.expectedContentLength > Int64(limit)
        completionHandler(oversized || self.response == nil ? .cancel : .allow)
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        guard !oversized else { return }
        guard data.count <= limit - body.count else {
            oversized = true
            dataTask.cancel()
            return
        }
        body.append(data)
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
        completionHandler(nil)
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        if oversized {
            code = -4
        } else if let error = error as? URLError {
            switch error.code {
            case .timedOut: code = -2
            case .notConnectedToInternet, .networkConnectionLost, .cannotFindHost, .cannotConnectToHost,
                 .dnsLookupFailed, .internationalRoamingOff, .dataNotAllowed: code = -1
            default: code = -3
            }
        } else if error == nil, let response {
            code = Int32(response.statusCode)
        }
        completed.signal()
    }
}

// Rust calls this on a blocking worker. The result bytes are valid only during the callback.
@_cdecl("dewclip_fetch_latest_release")
public func fetchLatestRelease(_ url: UnsafePointer<CChar>, _ etag: UnsafePointer<CChar>,
                               _ context: UnsafeMutableRawPointer?, _ callback: UpdateCallback) {
    guard let endpoint = URL(string: String(cString: url)),
          endpoint.scheme == "https" || endpoint.scheme == "http",
          endpoint.host != nil else {
        callback(context, -3, nil, 0, nil, nil, nil)
        return
    }
    let configuration = URLSessionConfiguration.ephemeral
    configuration.timeoutIntervalForRequest = 5
    configuration.timeoutIntervalForResource = 10
    configuration.urlCache = nil
    configuration.httpCookieStorage = nil
    configuration.httpShouldSetCookies = false
    configuration.urlCredentialStorage = nil
    let delegate = ReleaseResponse()
    let session = URLSession(configuration: configuration, delegate: delegate, delegateQueue: nil)
    defer { session.invalidateAndCancel() }

    var request = URLRequest(url: endpoint, timeoutInterval: 5)
    request.setValue("DewClip", forHTTPHeaderField: "User-Agent")
    request.setValue("", forHTTPHeaderField: "Accept-Language")
    request.setValue("application/vnd.github+json", forHTTPHeaderField: "Accept")
    request.setValue("2022-11-28", forHTTPHeaderField: "X-GitHub-Api-Version")
    let cached = String(cString: etag)
    if !cached.isEmpty { request.setValue(cached, forHTTPHeaderField: "If-None-Match") }
    session.dataTask(with: request).resume()
    guard delegate.completed.wait(timeout: .now() + 10) == .success else {
        callback(context, -2, nil, 0, nil, nil, nil)
        return
    }

    func header(_ name: String) -> String { delegate.response?.value(forHTTPHeaderField: name) ?? "" }
    header("ETag").withCString { etag in
        header("X-RateLimit-Reset").withCString { reset in
            header("Retry-After").withCString { retry in
                delegate.body.withUnsafeBytes { bytes in
                    callback(context, delegate.code, bytes.bindMemory(to: UInt8.self).baseAddress,
                             bytes.count, etag, reset, retry)
                }
            }
        }
    }
}
