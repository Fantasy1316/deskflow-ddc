#include "../../resources/vendor/deskflow/primary-entry.h"
#include <cassert>
#include <cstdio>
using namespace deskflow_ddc;
int main() {
    const Rect all{0,0,3432,1160}, retained{0,0,1512,982};
    for (const char side : {'l','r','t','b'}) {
        int x=3431, y=580;
        if (side=='r') { x=0; y=1100; }
        if (side=='t') { x=2400; y=1159; }
        if (side=='b') { x=2400; y=0; }
        assert(primaryEntry(all,retained,side,x,y));
        assert(contains(retained,x,y));
    }
    int x=3426,y=600;
    assert(primaryEntry(all,retained,'l',x,y));
    assert(x==1505 && y==508); // preserve the incoming fraction and overshoot
    int visibleX=1400,visibleY=300;
    assert(!primaryEntry(all,retained,'l',visibleX,visibleY));
    assert(visibleX==1400 && visibleY==300);
    int negativeX=1919,negativeY=439;
    assert(primaryEntry({-800,-400,2720,1680},{-800,-400,800,600},'l',negativeX,negativeY));
    assert(negativeX==-2 && negativeY>=-398 && negativeY<=198);
    int staleX=3000,staleY=600;
    assert(!primaryEntry(all,{5000,0,800,600},'l',staleX,staleY));
    assert(staleX==3000 && staleY==600);
    puts("primary entry geometry passed; no input posted");
}
