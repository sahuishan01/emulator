#include "replay_comparison.h"
#include <array>
#include <iostream>

void require(bool ok) { if (!ok) throw std::runtime_error("Replay evidence regression"); }
int main() {
    std::array<uint8_t,128> expected{};
    std::vector<uint8_t> actual(expected.size());
    auto result=replay_evidence::compare(actual,expected.data(),8,4);
    require(result.passed && result.json.find("\"mismatchedPixels\":0")!=std::string::npos && result.json.find("\"actualHex\"")==std::string::npos);
    actual[4*11+2]=255;
    result=replay_evidence::compare(actual,expected.data(),8,4);
    require(!result.passed && result.json.find("\"x\":3,\"y\":1")!=std::string::npos && result.json.find("\"mismatchedPixels\":1")!=std::string::npos && result.json.find("0000ff00")!=std::string::npos);
    for(size_t i=0;i<actual.size();i+=4)actual[i]=1;
    result=replay_evidence::compare(actual,expected.data(),8,4,true);
    require(!result.passed && result.json.find("d32-float-little-endian")!=std::string::npos && result.json.find("\"samplesTruncated\":true")!=std::string::npos && result.json.find("\"mismatchedPixels\":32")!=std::string::npos);
    size_t samples=0,position=0;
    while((position=result.json.find("\"x\":",position))!=std::string::npos){++samples;++position;}
    require(samples==8);
    actual.pop_back();
    result=replay_evidence::compare(actual,expected.data(),8,4);
    require(!result.passed && result.json.find("\"sizeMatches\":false")!=std::string::npos && result.json.find("\"mismatchedPixels\":null")!=std::string::npos);
    actual.resize(200);
    result=replay_evidence::compare(actual,expected.data(),8,4);
    require(!result.passed && result.json.find("\"actualTruncated\":true")!=std::string::npos);
    actual.clear();
    require(!replay_evidence::compare(actual,expected.data(),8,4).passed);
    for(auto dimensions:{std::array<uint32_t,2>{0,1},{65,1},{1,65}}){
        bool rejected=false;
        try {(void)replay_evidence::compare(actual,expected.data(),dimensions[0],dimensions[1]);}
        catch(const std::runtime_error&){rejected=true;}
        require(rejected);
    }
    std::cout<<"Nine exact replay evidence checks passed\n";
}
