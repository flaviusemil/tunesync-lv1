// dnssd.cpp - see dnssd.h.
#ifndef WIN32_LEAN_AND_MEAN
#  define WIN32_LEAN_AND_MEAN
#endif
#include <winsock2.h>
#include <windows.h>
#include <windns.h>
#include "dnssd.h"
#include <atomic>
#include <chrono>
#include <condition_variable>
#include <map>
#include <thread>

namespace trw {

namespace {

const wchar_t kService[] = L"_apple-midi._udp.local";
const char    kSuffix[]  = "._apple-midi._udp.local";

std::string narrow(const wchar_t* w) {
    if (!w) return std::string();
    char b[512] = {0};
    WideCharToMultiByte(CP_UTF8, 0, w, -1, b, sizeof b - 1, nullptr, nullptr);
    return b;
}
std::wstring wide(const std::string& s) {
    wchar_t b[512] = {0};
    MultiByteToWideChar(CP_UTF8, 0, s.c_str(), -1, b, 511);
    return b;
}
std::string lower(std::string s) { for (char& c : s) if (c >= 'A' && c <= 'Z') c = char(c - 'A' + 'a'); return s; }
std::string ip4(IP4_ADDRESS a) {
    const uint8_t* b = reinterpret_cast<const uint8_t*>(&a);   // network order
    return std::to_string(b[0]) + "." + std::to_string(b[1]) + "." + std::to_string(b[2]) + "." + std::to_string(b[3]);
}

struct Api {
    HMODULE dll = nullptr;
    decltype(&DnsServiceConstructInstance) construct = nullptr;
    decltype(&DnsServiceFreeInstance) freeInst = nullptr;
    decltype(&DnsServiceRegister) reg = nullptr;
    decltype(&DnsServiceDeRegister) dereg = nullptr;
    decltype(&DnsServiceBrowse) browse = nullptr;
    decltype(&DnsServiceBrowseCancel) browseCancel = nullptr;
    decltype(&DnsServiceResolve) resolve = nullptr;
    decltype(&DnsServiceResolveCancel) resolveCancel = nullptr;
    decltype(&DnsRecordListFree) recordFree = nullptr;
    bool ok() const { return construct && freeInst && reg && dereg && browse && browseCancel && resolve && recordFree; }
};

const Api& api() {
    static Api a = [] {
        Api x;
        x.dll = LoadLibraryW(L"dnsapi.dll");
        if (!x.dll) return x;
#define LOAD(field, name) x.field = reinterpret_cast<decltype(x.field)>(reinterpret_cast<void*>(GetProcAddress(x.dll, name)))
        LOAD(construct, "DnsServiceConstructInstance");
        LOAD(freeInst, "DnsServiceFreeInstance");
        LOAD(reg, "DnsServiceRegister");
        LOAD(dereg, "DnsServiceDeRegister");
        LOAD(browse, "DnsServiceBrowse");
        LOAD(browseCancel, "DnsServiceBrowseCancel");
        LOAD(resolve, "DnsServiceResolve");
        LOAD(resolveCancel, "DnsServiceResolveCancel");
        LOAD(recordFree, "DnsRecordListFree");
#undef LOAD
        return x;
    }();
    return a;
}

}  // namespace

// Owned by the thread and dnsapi's callbacks together. It is never freed: a callback
// dnsapi still owes us must find valid memory, and one State per start() is a few
// hundred bytes.
struct Dnssd::State {
    std::mutex              mu;
    std::condition_variable cv;
    bool                    stopping = false;
    std::thread             thr;
    LogFn                   log;
    std::string             name, hostFqdn;
    uint16_t                port = 0;
    std::string             advert = "starting";

    // registration
    PDNS_SERVICE_INSTANCE        inst = nullptr;
    DNS_SERVICE_REGISTER_REQUEST regReq{};
    bool                         registered = false;
    bool                         deregDone = false;

    // browsing: instance FQDN (lower case) -> what we know, and the round it was last seen
    struct Entry { FoundSession s; int seen = 0; bool resolving = false; std::string fqdn; };
    std::map<std::string, Entry> found;
    std::map<std::string, std::string> hostIp;   // host (lower) -> IPv4, from A records
    int                          round = 0;
    DNS_SERVICE_CANCEL           browseCancel{};
    bool                         browsing = false;
    std::vector<std::string>     toResolve;
    unsigned                     resolvesInFlight = 0;

    void say(const std::string& l, bool detail) { if (log) log(l, detail); }
};

namespace {

void WINAPI on_register(DWORD status, PVOID ctx, PDNS_SERVICE_INSTANCE inst) {
    auto* st = static_cast<Dnssd::State*>(ctx);
    if (inst) api().freeInst(inst);
    std::string line;
    {
        std::lock_guard<std::mutex> g(st->mu);
        if (st->stopping) {
            st->deregDone = true;
        } else if (status == ERROR_SUCCESS) {
            st->registered = true;
            st->advert = "advertised";
            line = "RTP-MIDI: advertised as \"" + st->name + "\" (_apple-midi._udp, port " + std::to_string(st->port) +
                   ") - Macs list it under MIDI Network Setup -> Directory";
        } else {
            st->advert = "failed (error " + std::to_string(status) + ")";
            line = "RTP-MIDI: advertising on the network failed (DNS-SD error " + std::to_string(status) +
                   "); on the Mac, add this host by its address instead";
        }
    }
    st->cv.notify_all();
    if (!line.empty()) st->say(line, false);
}

void WINAPI on_resolve(DWORD status, PVOID ctx, PDNS_SERVICE_INSTANCE inst) {
    auto* st = static_cast<Dnssd::State*>(ctx);
    {
        std::lock_guard<std::mutex> g(st->mu);
        if (st->resolvesInFlight) --st->resolvesInFlight;
        if (status == ERROR_SUCCESS && inst && inst->pszInstanceName) {
            const std::string fq = lower(narrow(inst->pszInstanceName));
            auto it = st->found.find(fq);
            if (it != st->found.end()) {
                it->second.resolving = false;
                if (inst->pszHostName) it->second.s.host = narrow(inst->pszHostName);
                if (inst->wPort) it->second.s.port = inst->wPort;
                if (inst->ip4Address) it->second.s.ip = ip4(*inst->ip4Address);
            }
        }
    }
    if (inst) api().freeInst(inst);
    st->cv.notify_all();
}

void WINAPI on_browse(DWORD status, PVOID ctx, PDNS_RECORD recs) {
    auto* st = static_cast<Dnssd::State*>(ctx);
    {
        std::lock_guard<std::mutex> g(st->mu);
        if (status != ERROR_SUCCESS) st->browsing = false;   // finished or cancelled
        for (PDNS_RECORD r = recs; r; r = r->pNext) {
            const std::string owner = lower(narrow(r->pName));
            if (r->wType == DNS_TYPE_PTR && r->Data.PTR.pNameHost) {
                const std::string fqName = narrow(r->Data.PTR.pNameHost);
                const std::string fq = lower(fqName);
                if (r->dwTtl == 0) { st->found.erase(fq); continue; }   // a goodbye
                auto& e = st->found[fq];
                e.fqdn = fqName;
                e.seen = st->round;
                if (e.s.instance.empty()) {
                    e.s.instance = fqName;
                    const size_t k = lower(fqName).rfind(kSuffix);
                    if (k != std::string::npos) e.s.instance = fqName.substr(0, k);
                }
            } else if (r->wType == DNS_TYPE_SRV && r->Data.SRV.pNameTarget) {
                auto it = st->found.find(owner);
                if (it != st->found.end()) {
                    it->second.s.host = narrow(r->Data.SRV.pNameTarget);
                    it->second.s.port = r->Data.SRV.wPort;
                }
            } else if (r->wType == DNS_TYPE_A) {
                st->hostIp[owner] = ip4(r->Data.A.IpAddress);
            }
        }
        for (auto& kv : st->found) {
            auto ip = st->hostIp.find(lower(kv.second.s.host));
            if (kv.second.s.ip.empty() && ip != st->hostIp.end()) kv.second.s.ip = ip->second;
            if ((kv.second.s.ip.empty() || !kv.second.s.port) && !kv.second.resolving) {
                kv.second.resolving = true;
                st->toResolve.push_back(kv.second.fqdn);
            }
        }
    }
    if (recs) api().recordFree(recs, DnsFreeRecordList);
    st->cv.notify_all();
}

std::string host_fqdn() {
    wchar_t b[256] = {0};
    DWORD n = 255;
    if (!GetComputerNameExW(ComputerNameDnsHostname, b, &n)) { n = 255; GetComputerNameW(b, &n); }
    return narrow(b) + ".local";
}

}  // namespace

bool Dnssd::available() const { return api().ok(); }

void Dnssd::start(const std::string& name, uint16_t port, LogFn log) {
    stop();
    st_ = new State;
    st_->log = std::move(log);
    st_->name = name;
    st_->port = port;
    st_->hostFqdn = host_fqdn();
    if (!api().ok()) {
        st_->advert = "not available on this Windows";
        st_->say("RTP-MIDI: this Windows has no DNS-SD (Windows 10 1809 or later): Macs won't list Tune Remote "
                 "by themselves; add this host by its address on the Mac", false);
        return;
    }
    State* st = st_;
    st->thr = std::thread([st] {
        const Api& a = api();
        // ---- advertise
        const std::wstring inst = wide(st->name) + L"." + kService;
        const std::wstring host = wide(st->hostFqdn);
        st->inst = a.construct(inst.c_str(), host.c_str(), nullptr, nullptr, st->port, 0, 0, 0, nullptr, nullptr);
        if (st->inst) {
            st->regReq.Version = DNS_QUERY_REQUEST_VERSION1;
            st->regReq.InterfaceIndex = 0;
            st->regReq.pServiceInstance = st->inst;
            st->regReq.pRegisterCompletionCallback = on_register;
            st->regReq.pQueryContext = st;
            st->regReq.unicastEnabled = FALSE;
            const DWORD r = a.reg(&st->regReq, nullptr);
            if (r != DNS_REQUEST_PENDING) {
                std::lock_guard<std::mutex> g(st->mu);
                st->advert = "failed (error " + std::to_string(r) + ")";
                st->say("RTP-MIDI: advertising on the network failed (DnsServiceRegister " + std::to_string(r) + ")", false);
            }
        }
        // ---- browse, again every 30 s: entries not seen for two rounds are dropped
        std::unique_lock<std::mutex> lk(st->mu);
        auto nextRound = std::chrono::steady_clock::now();
        while (!st->stopping) {
            if (std::chrono::steady_clock::now() >= nextRound) {
                if (st->browsing) {
                    lk.unlock();
                    a.browseCancel(&st->browseCancel);
                    lk.lock();
                    st->cv.wait_for(lk, std::chrono::milliseconds(500), [st] { return !st->browsing || st->stopping; });
                }
                ++st->round;
                for (auto it = st->found.begin(); it != st->found.end();)
                    it = st->round - it->second.seen > 2 ? st->found.erase(it) : std::next(it);
                DNS_SERVICE_BROWSE_REQUEST b{};
                b.Version = DNS_QUERY_REQUEST_VERSION1;
                b.InterfaceIndex = 0;
                b.QueryName = kService;
                b.pBrowseCallback = on_browse;
                b.pQueryContext = st;
                st->browseCancel = DNS_SERVICE_CANCEL{};
                lk.unlock();
                const DWORD r = a.browse(&b, &st->browseCancel);
                lk.lock();
                st->browsing = r == DNS_REQUEST_PENDING;
                if (!st->browsing && st->round == 1)
                    st->say("RTP-MIDI: browsing the network failed (DnsServiceBrowse " + std::to_string(r) + ")", true);
                nextRound = std::chrono::steady_clock::now() + std::chrono::seconds(30);
            }
            while (!st->toResolve.empty() && st->resolvesInFlight < 4) {
                const std::wstring q = wide(st->toResolve.back());
                st->toResolve.pop_back();
                DNS_SERVICE_RESOLVE_REQUEST rq{};
                rq.Version = DNS_QUERY_REQUEST_VERSION1;
                rq.QueryName = const_cast<PWSTR>(q.c_str());
                rq.pResolveCompletionCallback = on_resolve;
                rq.pQueryContext = st;
                DNS_SERVICE_CANCEL c{};
                ++st->resolvesInFlight;
                lk.unlock();
                const DWORD r = a.resolve(&rq, &c);
                lk.lock();
                if (r != DNS_REQUEST_PENDING && st->resolvesInFlight) --st->resolvesInFlight;
            }
            st->cv.wait_for(lk, std::chrono::milliseconds(500));
        }
        // ---- stop: cancel the browse, take the advertisement down
        const bool browsing = st->browsing;
        const bool registered = st->registered;
        lk.unlock();
        if (browsing) a.browseCancel(&st->browseCancel);
        if (registered && a.dereg(&st->regReq, nullptr) == DNS_REQUEST_PENDING) {
            lk.lock();
            st->cv.wait_for(lk, std::chrono::milliseconds(800), [st] { return st->deregDone; });
            lk.unlock();
        }
    });
}

void Dnssd::stop() {
    State* st = st_;
    if (!st) return;
    st_ = nullptr;
    {
        std::lock_guard<std::mutex> g(st->mu);
        st->stopping = true;
    }
    st->cv.notify_all();
    if (st->thr.joinable()) st->thr.join();
    // st is not freed: see State.
}

std::vector<FoundSession> Dnssd::found() const {
    std::vector<FoundSession> v;
    if (!st_) return v;
    std::lock_guard<std::mutex> g(st_->mu);
    for (const auto& kv : st_->found) {
        const FoundSession& s = kv.second.s;
        // ourselves, as the network sees us
        if (s.instance == st_->name && lower(s.host) == lower(st_->hostFqdn)) continue;
        v.push_back(s);
    }
    return v;
}

std::string Dnssd::advert_state() const {
    if (!st_) return "off";
    std::lock_guard<std::mutex> g(st_->mu);
    return st_->advert;
}

}  // namespace trw
