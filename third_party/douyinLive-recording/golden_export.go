package douyinLive

// Exported wrappers over private helpers, only in this recording copy, so
// rust-anything can record what they return. Results are flattened to strings.

import (
	"fmt"
	"strconv"
	"strings"
)

// GoldenWSStub is websocketSignatureParams.XMSStub.
func GoldenWSStub(roomID, userUniqueID string) string {
	return newWebsocketSignatureParams(roomID, userUniqueID).XMSStub()
}

// GoldenWSQuery is websocketURLParams.QueryString for the web profile.
func GoldenWSQuery(roomID, pushID, userAgent, cursor, internalExt, signature string, width, height int64) string {
	info := roomInfoSnapshot{roomID: roomID, pushID: pushID}
	p := newWebsocketURLParamsWithScreen(info, userAgent, cursor, internalExt, signature, int(width), int(height))
	p.PersistMsgCount = ProtocolModeWeb.persistMsgCount()
	return p.QueryString()
}

// GoldenIMFetchQuery is initialIMFetchParams.QueryString.
func GoldenIMFetchQuery(roomID, pushID, userAgent, msToken string, width, height int64) string {
	info := roomInfoSnapshot{roomID: roomID, pushID: pushID}
	return newInitialIMFetchParamsWithScreen(info, userAgent, msToken, int(width), int(height)).QueryString()
}

// GoldenRoomEnterQuery is buildRoomEnterParams. roomID is used only when liveKnown is true.
func GoldenRoomEnterQuery(liveID, roomID, userAgent, msToken string, liveKnown bool, width, height int64) string {
	dl := &DouyinLive{liveID: liveID, roomID: roomID}
	dl.userAgent = userAgent
	dl.msToken = msToken
	dl.fingerprint = browserFingerprint{ScreenWidth: int(width), ScreenHeight: int(height)}
	dl.cookieManager = nil
	if liveKnown {
		dl.liveStatusKnown = true
		dl.isLive = true
	}
	return dl.buildRoomEnterParams()
}

// GoldenInternalExt is defaultInternalExt.
func GoldenInternalExt(roomID, userUniqueID string, nowMs int64) string {
	return defaultInternalExt(roomID, userUniqueID, nowMs)
}

// GoldenPushURL is normalizeWebsocketPushURL.
func GoldenPushURL(candidate string) string {
	return normalizeWebsocketPushURL(candidate)
}

// GoldenPingInterval is parseHandshakePingInterval, in seconds.
func GoldenPingInterval(header string) int64 {
	return int64(parseHandshakePingInterval(header).Seconds())
}

// GoldenChromeVersion is chromeVersionFromUserAgent.
func GoldenChromeVersion(userAgent string) string {
	return chromeVersionFromUserAgent(userAgent)
}

// GoldenQueryEscape is queryEscapeValue and queryEscapeURLSearchParamsValue, joined by a tab.
func GoldenQueryEscape(value string) string {
	return queryEscapeValue(value) + "\t" + queryEscapeURLSearchParamsValue(value)
}

// GoldenValidateLiveID is ValidateLiveID.
func GoldenValidateLiveID(value string) (string, error) {
	return ValidateLiveID(value)
}

func snapshotText(info roomInfoSnapshot) string {
	return strings.Join([]string{info.roomID, info.pushID, info.liveName, info.title, info.avatarThumb, strconv.FormatBool(info.anchorOnly)}, "|")
}

// GoldenRoomEnter is parseRoomInfo and parseLiveStatusFromRoomEnter.
func GoldenRoomEnter(body string) (string, error) {
	info, err := parseRoomInfo(body)
	if err != nil {
		return "", err
	}
	live, known := parseLiveStatusFromRoomEnter(body)
	return fmt.Sprintf("%s|live=%t|known=%t", snapshotText(info), live, known), nil
}

// GoldenLivePage is parseLivePageState.
func GoldenLivePage(body string) string {
	s := parseLivePageState(body)
	return fmt.Sprintf("%s|uid=%s|anchor=%t|live=%t|known=%t|valid=%t",
		snapshotText(s.info), s.userUniqueID, s.hasAnchorIdentity, s.isLive, s.statusKnown, s.hasKnownPageIdentity())
}
