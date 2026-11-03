#pragma once
#include <algorithm>
#include <cstdint>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>

namespace replay_evidence {
struct Comparison { bool passed; std::string json; };

inline std::string hex(const uint8_t* data, size_t size) {
    constexpr char digits[] = "0123456789abcdef";
    std::string out;
    out.reserve(size * 2);
    for (size_t i = 0; i < size; ++i) {
        out += digits[data[i] >> 4];
        out += digits[data[i] & 15];
    }
    return out;
}

// Exact equality remains the acceptance rule. Failure evidence includes the
// complete bounded image, plus the first eight differing pixel coordinates.
inline Comparison compare(const std::vector<uint8_t>& actual, const uint8_t* expected,
                          uint32_t width, uint32_t height, bool depth = false) {
    if (!width || !height || width > 64 || height > 64)
        throw std::runtime_error("Replay evidence dimensions exceed bounds");
    const size_t bytes = static_cast<size_t>(width) * height * 4;
    const bool sized = actual.size() == bytes;
    size_t count = 0;
    std::ostringstream samples;
    if (sized) for (size_t p = 0; p < bytes / 4; ++p) {
        const auto at = p * 4;
        if (std::equal(actual.begin() + at, actual.begin() + at + 4, expected + at)) continue;
        if (count++ >= 8) continue;
        if (count != 1) samples << ',';
        samples << "{\"x\":" << p % width << ",\"y\":" << p / width
                << ",\"actualHex\":\"" << hex(actual.data() + at, 4)
                << "\",\"expectedHex\":\"" << hex(expected + at, 4) << "\"}";
    }
    const bool passed = sized && count == 0;
    std::ostringstream out;
    out << "{\"width\":" << width << ",\"height\":" << height
        << ",\"format\":\"" << (depth ? "d32-float-little-endian" : "attachment-color-bytes")
        << "\",\"actualBytes\":" << actual.size() << ",\"expectedBytes\":" << bytes
        << ",\"sizeMatches\":" << (sized ? "true" : "false")
        << ",\"mismatchedPixels\":";
    if (sized) out << count; else out << "null";
    out << ",\"samplesTruncated\":" << (count > 8 ? "true" : "false")
        << ",\"samples\":[" << samples.str() << ']';
    if (!passed) {
        const auto captured = std::min(actual.size(), bytes);
        out << ",\"actualHex\":\"" << hex(actual.data(), captured)
            << "\",\"expectedHex\":\"" << hex(expected, bytes)
            << "\",\"actualTruncated\":" << (actual.size() > bytes ? "true" : "false");
    }
    out << '}';
    return {passed, out.str()};
}
} // namespace replay_evidence
