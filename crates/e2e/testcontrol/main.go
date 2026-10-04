// Command testcontrol is crates/tailnet/testcontrol with every node owned by one
// user, as on a personal tailnet: the phones and the Mac's tag owner share a user
// ID, so the gate is exercised on pairing state and tags rather than ownership.
// It prints the control URL on stdout and serves until stdin closes.
package main

import (
	"crypto/tls"
	"flag"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"net/http/httptest"
	"os"

	"tailscale.com/derp/derpserver"
	"tailscale.com/tailcfg"
	"tailscale.com/tstest/integration/testcontrol"
	"tailscale.com/types/key"
	"tailscale.com/types/logger"
)

func main() {
	authKey := flag.String("authkey", "", "auth key every node must present")
	flag.Parse()
	if *authKey == "" {
		log.Fatal("-authkey is required")
	}

	derp := httptest.NewUnstartedServer(derpserver.Handler(derpserver.New(key.NewNode(), logger.Discard)))
	// DERP upgrades an HTTP/1.1 connection; HTTP/2 would break it.
	derp.Config.TLSNextProto = map[string]func(*http.Server, *tls.Conn, http.Handler){}
	derp.Config.ErrorLog = logger.StdLogger(logger.Discard)
	derp.StartTLS()
	defer derp.Close()

	control := &testcontrol.Server{
		DERPMap: &tailcfg.DERPMap{
			Regions: map[tailcfg.DERPRegionID]*tailcfg.DERPRegion{
				1: {
					RegionID:   1,
					RegionCode: "test",
					Nodes: []*tailcfg.DERPNode{{
						Name:             "t1",
						RegionID:         1,
						HostName:         "127.0.0.1",
						IPv4:             "127.0.0.1",
						IPv6:             "none",
						STUNPort:         -1,
						DERPPort:         derp.Listener.Addr().(*net.TCPAddr).Port,
						InsecureForTests: true,
					}},
				},
			},
		},
		RequireAuthKey:   *authKey,
		AllNodesSameUser: true,
		// Peers are reported online, as real control does for connected nodes; collie-core
		// does not dial a peer reported offline.
		AllOnline: true,
		// Non-nil so a node registering with RequestTags gets those tags.
		TagOwners: map[string][]string{},
		Logf:      logger.Discard,
	}
	control.HTTPTestServer = httptest.NewUnstartedServer(control)
	control.HTTPTestServer.Start()
	defer control.HTTPTestServer.Close()

	fmt.Println(control.HTTPTestServer.URL)
	io.Copy(io.Discard, os.Stdin)
}
