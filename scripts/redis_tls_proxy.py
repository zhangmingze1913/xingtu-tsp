
import socket, ssl, threading, sys
BASE=r"C:\Users\Administrator\Documents\deepseek-harness\default-workspace\xingtu-tsp\secrets"
ctx=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.load_cert_chain(BASE+r"\server.pem", BASE+r"\server-key.pem")
def pump(src,dst):
    try:
        while True:
            b=src.recv(65536)
            if not b: break
            dst.sendall(b)
    except Exception: pass
    finally:
        try: dst.shutdown(socket.SHUT_WR)
        except Exception: pass
def handle(c):
    try:
        s=ctx.wrap_socket(c,server_side=True)
        r=socket.create_connection(("127.0.0.1",56379),timeout=5)
        t1=threading.Thread(target=pump,args=(s,r),daemon=True); t1.start()
        pump(r,s)
    except Exception as e:
        print("proxy error",e,flush=True)
    finally:
        try: c.close()
        except Exception: pass
srv=socket.socket(); srv.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1); srv.bind(("127.0.0.1",56380)); srv.listen(50)
print("TLS redis proxy 56380 -> 56379",flush=True)
while True:
    c,_=srv.accept(); threading.Thread(target=handle,args=(c,),daemon=True).start()
