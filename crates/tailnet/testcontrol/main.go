// Command testcontrol runs tailscale's in-memory control server, a DERP relay
// and a STUN server on 127.0.0.1 for the integration tests. It prints the
// control URL on stdout and serves until stdin closes.
package main

import (
	"context"
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
	"tailscale.com/net/stunserver"
	"tailscale.com/tailcfg"
	"tailscale.com/tstest/integration/testcontrol"
	"tailscale.com/types/key"
	"tailscale.com/types/logger"
)

func main() {
	authKey := flag.String("authkey", "", "auth key every node must present")
	offline := flag.Bool("offline", false, "report every peer offline, as a stale netmap would")
	sameUser := flag.Bool("same-user", false, "own every node by one user")
	vmDir := flag.String("vm-dir", "", "private VM test marker directory; use fixed loopback ports")
	stunOnly := flag.Bool("stun-only", false, "serve only guest-local VM STUN")
	flag.Parse()
	if *vmDir != "" {
		var err error
		*authKey, err = vmAuthKey(*vmDir)
		if err != nil {
			log.Fatal(err)
		}
	} else if *stunOnly {
		log.Fatal("-stun-only requires -vm-dir")
	}
	if *authKey == "" {
		log.Fatal("-authkey is required")
	}

	// A local STUN response prevents repeated magicsock rebinds in the test tailnet.
	stunAddress := "127.0.0.1:0"
	if *vmDir != "" {
		stunAddress = "127.0.0.1:8791"
	}
	stun := stunserver.New(context.Background())
	if err := stun.Listen(stunAddress); err != nil {
		log.Fatal(err)
	}
	go stun.Serve()
	if *stunOnly {
		fmt.Println("VM STUN ready")
		io.Copy(io.Discard, os.Stdin)
		return
	}

	derp := httptest.NewUnstartedServer(derpserver.Handler(derpserver.New(key.NewNode(), logger.Discard)))
	// DERP upgrades an HTTP/1.1 connection; HTTP/2 would break it.
	derp.Config.TLSNextProto = map[string]func(*http.Server, *tls.Conn, http.Handler){}
	derp.Config.ErrorLog = logger.StdLogger(logger.Discard)
	if *vmDir != "" {
		derp.Listener.Close()
		listener, err := net.Listen("tcp", "127.0.0.1:8790")
		if err != nil {
			log.Fatal(err)
		}
		derp.Listener = listener
	}
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
						STUNPort:         stun.LocalAddr().(*net.UDPAddr).Port,
						DERPPort:         derp.Listener.Addr().(*net.TCPAddr).Port,
						InsecureForTests: true,
					}},
				},
			},
		},
		RequireAuthKey:   *authKey,
		AllNodesSameUser: *sameUser,
		// Peers are reported online, as real control does for connected nodes; collie-core
		// does not dial a peer reported offline.
		AllOnline: !*offline,
		// Non-nil so a node registering with RequestTags gets those tags.
		TagOwners: map[string][]string{},
		Logf:      logger.Discard,
	}
	control.HTTPTestServer = httptest.NewUnstartedServer(control)
	if *vmDir != "" {
		control.HTTPTestServer.Listener.Close()
		listener, err := net.Listen("tcp", "127.0.0.1:8789")
		if err != nil {
			log.Fatal(err)
		}
		control.HTTPTestServer.Listener = listener
	}
	control.HTTPTestServer.Start()
	defer control.HTTPTestServer.Close()

	fmt.Println(control.HTTPTestServer.URL)
	io.Copy(io.Discard, os.Stdin)
}
