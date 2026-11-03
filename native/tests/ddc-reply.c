#include <assert.h>
#include <stdio.h>
#include <string.h>
#include "ddc_reply.h"
static void checksum(uint8_t *data) { data[10]=0x50; for(int i=0;i<10;++i)data[10]^=data[i]; }
int main(void) {
 uint8_t reply[11]={0x6e,0x88,0x02,0,0x60,0,0x01,0x02,0,3,0};int cur=-1,max=-1;checksum(reply);
 assert(sb_parse_ddc_reply(reply,sizeof reply,0x60,&cur,&max));assert(cur==3&&max==258);
 reply[9]=0;checksum(reply);assert(sb_parse_ddc_reply(reply,sizeof reply,0x60,&cur,&max)&&cur==0);
 assert(!sb_parse_ddc_reply(reply,10,0x60,&cur,&max));
 assert(!sb_parse_ddc_reply(reply,11,0x10,&cur,&max));
 reply[3]=1;checksum(reply);assert(!sb_parse_ddc_reply(reply,11,0x60,&cur,&max));
 reply[3]=0;checksum(reply);reply[9]=1;assert(!sb_parse_ddc_reply(reply,11,0x60,&cur,&max));
 memset(reply,0,sizeof reply);assert(!sb_parse_ddc_reply(reply,11,0x60,&cur,&max));
 puts("DDC reply: valid, zero, endian, truncation, wrong feature, error, corruption and empty response checks passed");
}
